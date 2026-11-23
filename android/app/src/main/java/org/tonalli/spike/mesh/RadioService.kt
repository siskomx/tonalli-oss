package org.tonalli.spike.mesh

import android.app.*
import android.content.Intent
import android.os.*

/** User-started radio lifetime; never restarts a connection after process loss. */
class RadioService : Service() {
    private val handler = Handler(Looper.getMainLooper())
    private var observed = false
    private var started = 0L
    private val poll = object : Runnable {
        override fun run() {
            val active = MeshNative.active()
            observed = observed || active
            if ((!active && observed) || (!observed && SystemClock.elapsedRealtime() - started > 120_000)) stopSelf()
            else handler.postDelayed(this, 1000)
        }
    }
    override fun onCreate() {
        super.onCreate()
        val manager = getSystemService(NotificationManager::class.java)
        manager.createNotificationChannel(NotificationChannel("chat-radio", "Chat radio", NotificationManager.IMPORTANCE_LOW))
        val stop = PendingIntent.getService(this, 0, Intent(this, RadioService::class.java).setAction("stop"), PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE)
        val open = PendingIntent.getActivity(this, 0, Intent(this, org.tonalli.spike.MainActivity::class.java), PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE)
        startForeground(7210, Notification.Builder(this, "chat-radio").setSmallIcon(android.R.drawable.stat_sys_data_bluetooth)
            .setContentTitle("Tonalli · Chat radio").setContentText("Radio connection is enabled. Stop to release it.")
            .setContentIntent(open).setOngoing(true).addAction(Notification.Action.Builder(null, "Stop", stop).build()).build())
        started = SystemClock.elapsedRealtime()
        if (!MeshNative.initialize()) stopSelf() else handler.post(poll)
    }
    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        if (intent?.action == "stop") { MeshNative.stop(); stopSelf() }
        return START_NOT_STICKY
    }
    override fun onDestroy() { handler.removeCallbacks(poll); MeshNative.stop(); super.onDestroy() }
    override fun onBind(intent: Intent?) = null
}
