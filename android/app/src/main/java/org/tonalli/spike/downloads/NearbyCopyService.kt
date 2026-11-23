package org.tonalli.spike.downloads

import android.app.*
import android.content.Intent
import android.content.pm.ServiceInfo
import android.os.*
import java.util.concurrent.Executors
import java.util.concurrent.TimeUnit
import org.tonalli.spike.MainActivity
import org.tonalli.spike.R
import org.tonalli.spike.lan.LanBinding
import org.tonalli.spike.lan.LanNetwork

/** Non-sticky, user-started ownership; Activity destruction does not stop an accepted copy. */
class NearbyCopyService : Service() {
    private val worker = Executors.newSingleThreadScheduledExecutor()
    private val main = Handler(Looper.getMainLooper())
    @Volatile private var epoch = 0uL
    @Volatile private var latestStartId = 0
    @Volatile private var binding: LanBinding? = null
    private val wakeLock = Any()
    private var wake: PowerManager.WakeLock? = null
    private var renewWakeAt = 0L

    override fun onBind(intent: Intent): IBinder? = null
    override fun onCreate() {
        super.onCreate()
        worker.scheduleWithFixedDelay({ checkCopy() }, 1, 1, TimeUnit.SECONDS)
    }
    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        latestStartId = startId
        val attempt = intent?.getLongExtra(EPOCH, 0L)?.takeIf { it > 0 }?.toULong() ?: 0uL
        if (intent?.action == PAUSE) {
            NativeCopies.pause(attempt)
            if (epoch == 0uL) stopSelfResult(startId)
            return START_NOT_STICKY
        }
        if (intent?.action != START || attempt == 0uL) { stopSelfResult(startId); return START_NOT_STICKY }
        val selected = NativeCopies.take(attempt)
        if (selected == null) {
            if (epoch != attempt) stopSelfResult(startId)
            return START_NOT_STICKY
        }
        epoch = attempt; binding = selected
        try {
            val notifications = getSystemService(NotificationManager::class.java)
            notifications.createNotificationChannel(NotificationChannel(CHANNEL, getString(R.string.nearby_copy_channel), NotificationManager.IMPORTANCE_LOW))
            val pause = PendingIntent.getService(this, 0, Intent(this, NearbyCopyService::class.java)
                .setAction(PAUSE).setData(android.net.Uri.parse("tonalli-copy:attempt/$attempt"))
                .putExtra(EPOCH, attempt.toLong()), PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
            val open = PendingIntent.getActivity(this, 0, Intent(this, MainActivity::class.java)
                .setAction(Intent.ACTION_MAIN).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_CLEAR_TOP),
                PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
            startForeground(NOTIFICATION, Notification.Builder(this, CHANNEL).setSmallIcon(android.R.drawable.stat_sys_download)
                .setContentTitle(getString(R.string.nearby_copy_title)).setContentText(getString(R.string.nearby_copy_help))
                .setContentIntent(open).setOngoing(true).setVisibility(Notification.VISIBILITY_PRIVATE)
                .addAction(Notification.Action.Builder(null, getString(R.string.nearby_copy_pause), pause).build()).build(),
                ServiceInfo.FOREGROUND_SERVICE_TYPE_DATA_SYNC)
            renewWake(attempt, force = true)
        } catch (_: RuntimeException) {
            NativeCopies.pause(attempt)
            finishAttempt(attempt, startId)
        }
        return START_NOT_STICKY
    }
    private fun checkCopy() {
        val attempt = epoch; val startId = latestStartId
        if (attempt == 0uL) return
        try {
            val state = NativeCopies.state()
            if (state == null || state.epoch != attempt || !state.running) { finishAttempt(attempt, startId); return }
            val selected = binding
            if (selected == null || !LanNetwork(this).contains(selected)) NativeCopies.pause(attempt)
            renewWake(attempt)
        } catch (_: Exception) { NativeCopies.pause(attempt); finishAttempt(attempt, startId) }
    }
    private fun renewWake(attempt: ULong, force: Boolean = false) = synchronized(wakeLock) {
        if (epoch != attempt || attempt == 0uL) return@synchronized
        if (force || SystemClock.elapsedRealtime() >= renewWakeAt) {
            if (wake == null) wake = getSystemService(PowerManager::class.java)
                .newWakeLock(PowerManager.PARTIAL_WAKE_LOCK, "tonalli:nearby-copy").apply { setReferenceCounted(false) }
            wake?.acquire(TimeUnit.MINUTES.toMillis(30))
            renewWakeAt = SystemClock.elapsedRealtime() + TimeUnit.MINUTES.toMillis(25)
        }
    }
    private fun releaseWake() = synchronized(wakeLock) {
        wake?.let { if (it.isHeld) it.release() }; wake = null
    }
    private fun finishAttempt(attempt: ULong, startId: Int) {
        main.post {
            if (epoch == attempt && stopSelfResult(startId)) {
                epoch = 0uL; binding = null
                stopForeground(STOP_FOREGROUND_REMOVE)
                releaseWake()
            }
        }
    }
    override fun onDestroy() {
        val attempt = epoch; epoch = 0uL; binding = null
        NativeCopies.pause(attempt)
        worker.shutdownNow()
        releaseWake()
        super.onDestroy()
    }
    companion object {
        const val START = "org.tonalli.spike.START_NEARBY_COPY"
        const val PAUSE = "org.tonalli.spike.PAUSE_NEARBY_COPY"
        const val EPOCH = "copyEpoch"
        private const val CHANNEL = "nearby-copies"
        private const val NOTIFICATION = 13
    }
}
