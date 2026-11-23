package org.tonalli.meshspike

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Intent
import android.content.pm.ServiceInfo
import android.os.IBinder
import java.io.File
import java.util.concurrent.atomic.AtomicBoolean

class SpikeService : Service() {
    private val running = AtomicBoolean(false)
    @Volatile private var latestStartId = 0
    override fun onBind(intent: Intent?): IBinder? = null
    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        latestStartId = startId
        if (intent == null) { stopSelf(); return START_NOT_STICKY }
        if (intent?.action == "org.tonalli.meshspike.STOP") { stopSelf(); return START_NOT_STICKY }
        if (!running.compareAndSet(false, true)) return START_NOT_STICKY
        val manager = getSystemService(NotificationManager::class.java)
        manager.createNotificationChannel(NotificationChannel("mesh-spike", "Mesh spike", NotificationManager.IMPORTANCE_LOW))
        val stop = PendingIntent.getService(this, 1, Intent(this, SpikeService::class.java).setAction("org.tonalli.meshspike.STOP"), PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
        startForeground(1, Notification.Builder(this, "mesh-spike").setSmallIcon(android.R.drawable.stat_sys_data_bluetooth)
            .setContentTitle("Tonalli Mesh spike").setContentText("One radio session; use Stop to end it")
            .setOngoing(true).addAction(Notification.Action.Builder(null, "Stop", stop).build()).build(), ServiceInfo.FOREGROUND_SERVICE_TYPE_CONNECTED_DEVICE)
        val console = File(filesDir, "mesh-console.txt")
        val config = File(filesDir, "mesh-spike.toml")
        val source = try { if (config.isFile && config.length() <= 65536) config.readText() else null } catch (_: Exception) { null }
        if (source == null || !MeshNative.initialize()) {
            console.writeText("error=Configuration or Bluetooth initialization unavailable\n")
            stopSelf(); return START_NOT_STICKY
        }
        console.writeText("connecting; no result yet\n")
        Thread({
            val report = try { MeshNative.run(source, true) } catch (_: Exception) { "error=Mesh worker unavailable\n" }
            console.writeText(report)
            stopSelfResult(latestStartId)
        }, "MeshSpike").start()
        return START_NOT_STICKY
    }
    override fun onDestroy() {
        MeshNative.stop()
        super.onDestroy()
    }
}
