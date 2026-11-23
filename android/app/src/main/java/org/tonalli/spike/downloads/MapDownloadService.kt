package org.tonalli.spike.downloads

import android.app.Application
import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.job.JobParameters
import android.app.job.JobService
import android.content.Intent
import android.net.ConnectivityManager
import android.net.NetworkCapabilities
import android.os.Handler
import android.os.Looper
import java.util.concurrent.Executors
import java.util.concurrent.atomic.AtomicBoolean
import org.tonalli.spike.MainActivity
import org.tonalli.spike.R
import uniffi.tonalli_core.*

/** Map UIDT has its own process; Wi-Fi routing cannot affect the reader or other workers. */
class MapDownloadService : JobService() {
    private class Task(val params: JobParameters, val ticket: MapTicket) {
        val stopped = AtomicBoolean(false)
        val lock = Any()
        var owner: MapsHandle? = null
    }
    private val tasks = mutableMapOf<String, Task>()
    private val worker = Executors.newSingleThreadExecutor()
    private val control = Executors.newSingleThreadExecutor()
    private val main = Handler(Looper.getMainLooper())
    private fun key(ticket: MapTicket) = "${ticket.id}:${ticket.epoch}"
    override fun onStartJob(params: JobParameters): Boolean {
        val ticket = NativeMaps.ticket(params.extras) ?: return false
        if (params.jobId != NativeMaps.JOB_ID || Application.getProcessName() != "$packageName:mapdownloads") return false
        val task = Task(params, ticket)
        if (tasks.putIfAbsent(key(ticket), task) != null) return false
        val notifications = getSystemService(NotificationManager::class.java)
        notifications.createNotificationChannel(NotificationChannel("map-downloads", getString(R.string.content_download_channel), NotificationManager.IMPORTANCE_LOW))
        val open = PendingIntent.getActivity(this, 0, Intent(this, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_CLEAR_TOP), PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE)
        setNotification(params, NativeMaps.JOB_ID, Notification.Builder(this, "map-downloads").setSmallIcon(android.R.drawable.stat_sys_download)
            .setContentTitle(getString(R.string.content_download_title)).setContentText(getString(R.string.content_download_running))
            .setOngoing(true).setContentIntent(open).build(), JOB_END_NOTIFICATION_POLICY_REMOVE)
        worker.execute { run(task) }
        return true
    }
    private fun run(task: Task) {
        val connectivity = getSystemService(ConnectivityManager::class.java)
        var run: MapRunHandle? = null
        var bound = false
        var unbound = true
        try {
            val owner = NativeMaps.open(applicationContext)
            synchronized(task.lock) { task.owner = owner }
            run = mapsClaim(owner, task.ticket)
            if (task.stopped.get()) mapsPause(owner, task.ticket)
            val network = task.params.network
            val capabilities = network?.let(connectivity::getNetworkCapabilities)
            bound = !task.stopped.get() && network != null && capabilities != null && capabilities.hasTransport(NetworkCapabilities.TRANSPORT_WIFI) &&
                capabilities.hasCapability(NetworkCapabilities.NET_CAPABILITY_INTERNET) && connectivity.bindProcessToNetwork(network)
            if (bound) mapsExecute(run) else mapsPause(owner, task.ticket)
        } catch (_: Exception) { /* Core owns the persisted result. */ }
        finally {
            if (bound) unbound = try { connectivity.bindProcessToNetwork(null) } catch (_: Exception) { false }
            if (run != null) {
                if (unbound) try { mapsFinish(run) } catch (_: PackException) { }
                run.destroy()
            }
            synchronized(task.lock) { task.owner?.destroy(); task.owner = null }
            main.post {
                if (tasks[key(task.ticket)] === task) tasks.remove(key(task.ticket))
                val pending = getSystemService(android.app.job.JobScheduler::class.java).getPendingJob(NativeMaps.JOB_ID)
                val pendingTicket = pending?.let { NativeMaps.ticket(it.extras) }
                if (pendingTicket != null && pendingTicket.id == task.ticket.id && pendingTicket.epoch == task.ticket.epoch) jobFinished(task.params, false)
            }
        }
    }
    private fun stop(task: Task) {
        task.stopped.set(true)
        control.execute { synchronized(task.lock) { task.owner?.let { try { mapsPause(it, task.ticket) } catch (_: PackException) { } } } }
    }
    override fun onStopJob(params: JobParameters): Boolean {
        NativeMaps.ticket(params.extras)?.let { tasks[key(it)]?.let(::stop) }
        return false
    }
    override fun onDestroy() {
        tasks.values.forEach(::stop)
        worker.shutdown(); control.shutdown(); super.onDestroy()
    }
}
