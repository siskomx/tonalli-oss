package org.tonalli.spike.tv

import android.app.*
import android.content.*
import android.content.pm.ServiceInfo
import android.net.wifi.WifiManager
import android.os.*
import java.util.concurrent.*
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicLong
import org.tonalli.spike.R
import org.tonalli.spike.lan.LanBinding
import org.tonalli.spike.lan.LanLifecycle
import org.tonalli.spike.lan.LanNetwork
import uniffi.tonalli_core.*

/** Explicit non-sticky TV sharing; only the selected catalog reaches this owner. */
class TvService : Service() {
    private val worker = Executors.newSingleThreadScheduledExecutor()
    private val generation = AtomicLong()
    private val requested = AtomicBoolean()
    private val lifecycle = LanLifecycle()
    private val main = Handler(Looper.getMainLooper())
    private lateinit var network: LanNetwork
    private var binding: LanBinding? = null
    private var handle: TvHandle? = null
    private var wake: PowerManager.WakeLock? = null
    private var renewWakeAt = 0L
    private var requestId: String? = null
    @Volatile private var latestStartId = 0
    private var receiverRegistered = false
    private val battery = object : BroadcastReceiver() {
        override fun onReceive(context: Context, intent: Intent) {
            if (intent.action == Intent.ACTION_BATTERY_LOW) stopTv(getString(R.string.tv_low_battery))
        }
    }
    override fun onBind(intent: Intent): IBinder? = null
    override fun onCreate() {
        super.onCreate(); NativeTv.service = this; network = LanNetwork(this)
        registerReceiver(battery, IntentFilter(Intent.ACTION_BATTERY_LOW), RECEIVER_NOT_EXPORTED); receiverRegistered = true
        worker.scheduleWithFixedDelay({
            if (requested.get() && binding != null) {
                if (!runCatching { network.contains(checkNotNull(binding)) }.getOrDefault(false)) stopTv(getString(R.string.tv_network_lost))
                else runCatching { update() }.onFailure { stopTv(getString(R.string.tv_network_lost)) }
            }
        }, 2, 2, TimeUnit.SECONDS)
    }
    private fun publish(value: TvStatus, epoch: Long) {
        if (epoch == generation.get() && NativeTv.service === this) NativeTv.snapshot = value
    }
    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        latestStartId = startId
        if (intent?.action == STOP) { stopTv(null); return START_NOT_STICKY }
        if (intent?.action != START) { stopSelf(startId); return START_NOT_STICKY }
        if (!requested.compareAndSet(false, true)) return START_NOT_STICKY
        requestId = intent.getStringExtra("catalogRequest")
        try {
            getSystemService(NotificationManager::class.java).createNotificationChannel(NotificationChannel(CHANNEL, getString(R.string.tv_channel), NotificationManager.IMPORTANCE_LOW))
            val stop = PendingIntent.getService(this, 0, Intent(this, TvService::class.java).setAction(STOP), PendingIntent.FLAG_IMMUTABLE)
            val open = PendingIntent.getActivity(this, 0, Intent(this, org.tonalli.spike.MainActivity::class.java), PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
            startForeground(11, Notification.Builder(this, CHANNEL).setSmallIcon(android.R.drawable.stat_sys_upload)
                .setContentTitle(getString(R.string.tv_notification_title, applicationInfo.loadLabel(packageManager)))
                .setContentText(getString(R.string.tv_notification_help)).setContentIntent(open).setOngoing(true)
                .addAction(Notification.Action.Builder(null, getString(R.string.tv_stop), stop).build()).build(), ServiceInfo.FOREGROUND_SERVICE_TYPE_SPECIAL_USE)
        } catch (failure: RuntimeException) {
            android.util.Log.e("TonalliTv", "Foreground startup failed", failure)
            stopTv(getString(R.string.tv_start_failed)); return START_NOT_STICKY
        }
        val epoch = generation.incrementAndGet()
        val id = requestId
        worker.execute {
            if (epoch != generation.get()) return@execute
            val request = id?.let { NativeTv.take(it) }
            if (request == null) { stopTv(null); return@execute }
            try {
                val low = registerReceiver(null, IntentFilter(Intent.ACTION_BATTERY_CHANGED))?.getBooleanExtra(BatteryManager.EXTRA_BATTERY_LOW, false) ?: false
                if (low) { stopTv(getString(R.string.tv_low_battery)); return@execute }
                val token = lifecycle.begin(false) ?: throw IllegalStateException("TV startup unavailable")
                val bound = checkNotNull(network.wifiBinding(ipv4Only = true))
                val multicast = getSystemService(WifiManager::class.java).createMulticastLock("tonalli-tv")
                multicast.setReferenceCounted(false); multicast.acquire()
                if (!lifecycle.adopt(token, AutoCloseable { if (multicast.isHeld) multicast.release() })) return@execute
                val lock = getSystemService(PowerManager::class.java).newWakeLock(PowerManager.PARTIAL_WAKE_LOCK, "tonalli:tv")
                lock.setReferenceCounted(false); lock.acquire(TimeUnit.MINUTES.toMillis(30))
                if (!lifecycle.adopt(token, AutoCloseable { if (lock.isHeld) lock.release() })) return@execute
                wake = lock; renewWakeAt = SystemClock.elapsedRealtime() + TimeUnit.MINUTES.toMillis(25)
                val native = serverStartTv(request.catalog, bound.address, 0u)
                val owned = AutoCloseable { try { serverStopTv(native) } finally { native.destroy() } }
                if (epoch != generation.get()) { owned.close(); return@execute }
                if (!lifecycle.adopt(token, owned)) return@execute
                handle = native; binding = bound
                if (lifecycle.complete(token)) update()
            } catch (failure: Exception) {
                android.util.Log.e("TonalliTv", "TV startup failed", failure)
                if (epoch == generation.get()) stopTv(getString(R.string.tv_start_failed))
            } finally { request.close() }
        }
        return START_NOT_STICKY
    }
    private fun update() {
        val epoch = generation.get()
        if (!requested.get()) return
        val info = tvInfo(checkNotNull(handle))
        if (SystemClock.elapsedRealtime() >= renewWakeAt) {
            wake?.acquire(TimeUnit.MINUTES.toMillis(30)); renewWakeAt = SystemClock.elapsedRealtime() + TimeUnit.MINUTES.toMillis(25)
        }
        publish(TvStatus(true, false, info.url, emptyList(), null, NativeTv.snapshot.selectedMedia), epoch)
    }
    internal fun stopAndWait() {
        val done = CountDownLatch(1); stopTv(null, done)
        check(done.await(5, TimeUnit.SECONDS)) { "TV mode is still stopping" }
    }
    internal fun requestStop() = stopTv(null)
    override fun onTaskRemoved(rootIntent: Intent?) {
        stopTv(null)
        super.onTaskRemoved(rootIntent)
    }
    private fun stopTv(message: String?, done: CountDownLatch? = null) {
        if (NativeTv.service === this) NativeTv.cancelPending()
        requested.set(false)
        val epoch = generation.incrementAndGet(); val stopId = latestStartId
        try { worker.execute {
            handle = null; binding = null; wake = null
            runCatching { lifecycle.stop() }
            publish(TvStatus(false, false, null, emptyList(), message, emptyList()), epoch)
            done?.countDown()
            main.post { if (epoch == generation.get() && stopSelfResult(stopId)) stopForeground(STOP_FOREGROUND_REMOVE) }
        } } catch (_: RejectedExecutionException) { done?.countDown() }
    }
    override fun onDestroy() {
        generation.incrementAndGet(); requested.set(false)
        NativeTv.cancelPending(requestId)
        if (NativeTv.service === this) {
            NativeTv.service = null
            NativeTv.snapshot = NativeTv.snapshot.copy(running = false, busy = true, url = null)
        }
        if (receiverRegistered) unregisterReceiver(battery)
        worker.execute {
            handle = null; binding = null; wake = null; runCatching { lifecycle.stop() }
            if (NativeTv.service == null && !NativeTv.hasPending()) NativeTv.snapshot = NativeTv.snapshot.copy(running = false, busy = false, url = null, selectedMedia = emptyList())
        }
        worker.shutdown(); super.onDestroy()
    }
    companion object {
        const val START = "org.tonalli.spike.START_TV"
        const val STOP = "org.tonalli.spike.STOP_TV"
        private const val CHANNEL = "tonalli-tv"
    }
}
