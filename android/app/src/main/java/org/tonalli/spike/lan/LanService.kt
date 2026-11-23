package org.tonalli.spike.lan

import android.app.*
import android.content.*
import android.content.pm.ServiceInfo
import android.net.wifi.WifiManager
import android.os.*
import java.util.concurrent.Executors
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicLong
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.RejectedExecutionException
import uniffi.tonalli_core.*

/** Explicit, non-sticky LAN owner. All native operations and leases share one worker. */
class LanService : Service() {
    private val worker = Executors.newSingleThreadScheduledExecutor()
    private val generation = AtomicLong()
    private val requested = AtomicBoolean(false)
    private val lifecycle = LanLifecycle()
    private val main = Handler(Looper.getMainLooper())
    private lateinit var network: LanNetwork
    private var session: LanSession? = null
    private var binding: LanBinding? = null
    @Volatile private var latestStartId = 0
    private var receiverRegistered = false
    @Volatile private var report = "LAN server is off"
    inner class Controls : Binder() {
        fun status(): String = report
        fun refresh() { worker.execute { updateReport() } }
        fun kick(id: String) { worker.execute { runCatching { session?.let { lanKick(it.handle, id) }; updateReport() } } }
    }
    private val controls = Controls()
    private val battery = object : BroadcastReceiver() {
        override fun onReceive(context: Context, intent: Intent) {
            if (intent.action == Intent.ACTION_BATTERY_LOW) stopLan("Stopped: system low battery")
        }
    }

    override fun onCreate() {
        super.onCreate()
        NativeSharing.service = this
        network = LanNetwork(this)
        registerReceiver(battery, IntentFilter(Intent.ACTION_BATTERY_LOW), RECEIVER_NOT_EXPORTED)
        receiverRegistered = true
        worker.scheduleWithFixedDelay({
            val bound = binding
            if (bound != null) {
                if (runCatching { network.contains(bound) }.getOrDefault(false).not())
                    stopLan("Stopped: Wi-Fi or hotspot address lost")
                else updateReport()
            }
        }, 2, 2, TimeUnit.SECONDS)
    }

    override fun onBind(intent: Intent): IBinder = controls

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        latestStartId = startId
        if (intent?.action == STOP) { stopLan("LAN server is off"); return START_NOT_STICKY }
        if (intent?.action != START) { stopSelf(startId); return START_NOT_STICKY }
        if (!requested.compareAndSet(false, true)) return START_NOT_STICKY
        // Show the Stop action before any native startup or hotspot callback.
        try {
            val notifications = getSystemService(NotificationManager::class.java)
            notifications.createNotificationChannel(NotificationChannel(CHANNEL, "LAN sharing", NotificationManager.IMPORTANCE_LOW))
            val stop = PendingIntent.getService(this, 0, Intent(this, LanService::class.java).setAction(STOP), PendingIntent.FLAG_IMMUTABLE)
            val openIntent = if (intent.hasExtra("libraryRequest")) Intent(this, org.tonalli.spike.web.WebProbeActivity::class.java)
                .setAction("org.tonalli.spike.RESUME_SHARING") else Intent(this, LanProbeActivity::class.java)
            val open = PendingIntent.getActivity(this, 1, openIntent, PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
            startForeground(10, Notification.Builder(this, CHANNEL).setSmallIcon(android.R.drawable.stat_sys_upload)
                .setContentTitle("${applicationInfo.loadLabel(packageManager)} LAN sharing")
                .setContentText("Chosen library content only. Tap Stop to end sharing.")
                .setContentIntent(open).setOngoing(true).addAction(Notification.Action.Builder(null, "Stop", stop).build()).build(),
                ServiceInfo.FOREGROUND_SERVICE_TYPE_SPECIAL_USE)
        } catch (_: RuntimeException) {
            stopLan("Android refused foreground sharing; check app permissions and retry while the app is visible")
            return START_NOT_STICKY
        }
        val epoch = generation.incrementAndGet()
        val hotspot = intent.getBooleanExtra("hotspot", false)
        val wake = intent.getBooleanExtra("wake", false)
        val libraryId = intent.getStringExtra("libraryRequest")
        worker.execute {
            if (epoch != generation.get()) return@execute
            val low = registerReceiver(null, IntentFilter(Intent.ACTION_BATTERY_CHANGED))
                ?.getBooleanExtra(BatteryManager.EXTRA_BATTERY_LOW, false) ?: false
            val token = lifecycle.begin(low)
            if (token == null) {
                if (low) stopLan("Refused: system low battery")
                return@execute
            }
            report = "Starting LAN sharing"
            try {
                if (hotspot) startHotspot(token, epoch, wake, libraryId)
                else {
                    val request = libraryId?.let { NativeSharing.take(it) }
                    if (libraryId != null && request == null) { stopLan("Sharing cancelled"); return@execute }
                    try {
                        startNative(token, epoch, checkNotNull(network.wifiBinding()) { "No private Wi-Fi address" }, wake, request = request)
                    } finally { request?.close() }
                }
            } catch (_: Exception) {
                if (epoch == generation.get()) stopLan("LAN startup failed; check Wi-Fi, permissions and chosen content")
            }
        }
        return START_NOT_STICKY
    }

    private fun startHotspot(token: Long, epoch: Long, wake: Boolean, libraryId: String?) {
        val before = network.interfaces()
        try { getSystemService(WifiManager::class.java).startLocalOnlyHotspot(object : WifiManager.LocalOnlyHotspotCallback() {
            override fun onStarted(reservation: WifiManager.LocalOnlyHotspotReservation) {
                try { worker.execute {
                    if (epoch != generation.get()) { reservation.close(); return@execute }
                    if (!lifecycle.adopt(token, AutoCloseable { reservation.close() })) return@execute
                    val config = reservation.softApConfiguration
                    val credentials = "Hotspot: ${config?.ssid}\nPassword: ${config?.passphrase}"
                    val ssid = config?.ssid
                    val password = config?.passphrase
                    if (ssid != null && password != null) hotspotInfo = SharingHotspot(ssid, password)
                    awaitHotspotAddress(token, epoch, before, wake, credentials, libraryId, 0)
                } } catch (_: RejectedExecutionException) { reservation.close() }
            }
            override fun onStopped() { if (epoch == generation.get()) stopLan("Hotspot stopped") }
            override fun onFailed(reason: Int) { if (epoch == generation.get()) stopLan("Local-only hotspot unavailable ($reason)") }
        }, main) } catch (_: SecurityException) {
            if (epoch == generation.get()) stopLan("Local-only hotspot permission was not granted")
        }
    }

    private fun awaitHotspotAddress(token: Long, epoch: Long, before: Set<String>, wake: Boolean, credentials: String, libraryId: String?, attempt: Int) {
        if (epoch != generation.get()) return
        try {
            val bind = network.hotspotBinding(before)
            if (bind != null) {
                val request = libraryId?.let { NativeSharing.take(it) }
                if (libraryId != null && request == null) { stopLan("Sharing cancelled"); return }
                try { startNative(token, epoch, bind, wake, credentials, request) } finally { request?.close() }
                return
            }
            if (attempt >= 20) { stopLan("No unambiguous private hotspot address"); return }
            worker.schedule({ awaitHotspotAddress(token, epoch, before, wake, credentials, libraryId, attempt + 1) }, 500, TimeUnit.MILLISECONDS)
        } catch (_: Exception) { if (epoch == generation.get()) stopLan("Hotspot address unavailable") }
    }

    private var hotspotText = ""
    @Volatile private var hotspotInfo: SharingHotspot? = null
    private fun startNative(token: Long, epoch: Long, bind: LanBinding, wake: Boolean, hotspot: String = "", request: NativeSharing.Request? = null) {
        if (epoch != generation.get()) return
        val multicast = getSystemService(WifiManager::class.java).createMulticastLock("tonalli-mdns")
        multicast.setReferenceCounted(false)
        multicast.acquire()
        if (!lifecycle.adopt(token, AutoCloseable { if (multicast.isHeld) multicast.release() })) return
        if (wake) {
            val lock = getSystemService(PowerManager::class.java).newWakeLock(PowerManager.PARTIAL_WAKE_LOCK, "tonalli:lan")
            lock.acquire(TimeUnit.MINUTES.toMillis(30))
            if (!lifecycle.adopt(token, AutoCloseable { if (lock.isHeld) lock.release() })) return
        }
        val native = if (request != null) LanSession.fromLibrary(this, bind.address, request) else LanSession.start(this, bind.address)
        if (epoch != generation.get()) { native.close(); return }
        if (!lifecycle.adopt(token, native)) return
        session = native
        binding = bind
        hotspotText = hotspot
        if (lifecycle.complete(token)) updateReport()
    }

    private fun updateReport() {
        val native = session ?: return
        try {
            val info = lanInfo(native.handle)
            NativeSharing.snapshot = SharingStatus(true, false, info.url, lanPairingCode(native.handle),
                lanDevices(native.handle).map { SharingDevice(it.id, it.label) }, emptyList(), null, info.certificateSha256, info.friendlyUrl, info.nativeInvitation, hotspotInfo)
            report = listOf(hotspotText, info.url, "SPKI SHA256: ${info.spkiSha256}",
                "Pairing code: ${lanPairingCode(native.handle)}", "Devices:",
                lanDevices(native.handle).joinToString("\n") { "${it.id} | ${it.label} | ${it.peerAddress}" }).filter { it.isNotEmpty() }.joinToString("\n")
        } catch (_: Exception) { stopLan("LAN session unavailable") }
    }

    internal fun disconnect(id: String) { worker.execute { session?.let { lanKick(it.handle, id) }; updateReport() } }

    internal fun room(request: RoomRequest): RoomSnapshot {
        val epoch=generation.get()
        fun off():RoomSnapshot {
            if(request.action==RoomAction.GET||request.action==RoomAction.LEAVE)return RoomSnapshot(false,"","",false,0UL,emptyList(),emptyList())
            throw RoomException.Unavailable()
        }
        val task=try { worker.submit<RoomSnapshot> {
            if(epoch!=generation.get()||!requested.get())off()
            else session?.let { lanRoom(it.handle,request) } ?: off()
        } } catch(_:RejectedExecutionException) { throw RoomException.Unavailable() }
        try { return task.get(2,TimeUnit.SECONDS) }
        catch(failure:java.util.concurrent.ExecutionException) { throw (failure.cause as? RoomException ?: RoomException.Unavailable()) }
        catch(_:Exception) { task.cancel(false);throw RoomException.Unavailable() }
    }

    internal fun stopAndWait() {
        val done = java.util.concurrent.CountDownLatch(1)
        stopLan("LAN server is off", done)
        check(done.await(5, TimeUnit.SECONDS)) { "Sharing is still stopping" }
    }

    private fun stopLan(message: String, done: java.util.concurrent.CountDownLatch? = null) {
        NativeSharing.cancelPending()
        requested.set(false)
        val epoch = generation.incrementAndGet()
        val stopId = latestStartId
        worker.execute {
            session = null
            binding = null
            hotspotText = ""
            hotspotInfo = null
            runCatching { lifecycle.stop() }
            report = message
            NativeSharing.snapshot = SharingStatus(false, false, null, null, emptyList(), emptyList(),
                if (message == "LAN server is off") null else message, null, null, null, null)
            done?.countDown()
            main.post {
                // Android can have a newer queued start that our generation has not seen yet.
                if (epoch == generation.get() && stopSelfResult(stopId)) stopForeground(STOP_FOREGROUND_REMOVE)
            }
        }
    }

    override fun onDestroy() {
        if (NativeSharing.service === this) NativeSharing.service = null
        generation.incrementAndGet()
        requested.set(false)
        if (receiverRegistered) unregisterReceiver(battery)
        worker.execute { session = null; binding = null; hotspotText = ""; hotspotInfo = null; runCatching { lifecycle.stop() }; report = "LAN server is off"
            NativeSharing.snapshot = SharingStatus(false, false, null, null, emptyList(), emptyList(), null, null, null, null, null) }
        // Delayed callbacks still see a stale epoch and close any delivered reservation.
        worker.shutdown()
        super.onDestroy()
    }

    companion object {
        const val START = "org.tonalli.spike.START_LAN"
        const val STOP = "org.tonalli.spike.STOP_LAN"
        private const val CHANNEL = "tonalli-lan"
    }
}
