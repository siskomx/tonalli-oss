package org.tonalli.spike.downloads

import android.app.Application
import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.job.JobParameters
import android.app.job.JobService
import android.app.job.JobScheduler
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

/** Dedicated process: binding the selected Wi-Fi never reroutes the UI or local sharing. */
class ContentDownloadService : JobService() {
    private class Task(val params: JobParameters, val ticket: ContentTicket) {
        val stopped = AtomicBoolean(false)
        val lock = Any()
        var owner: ContentHandle? = null
    }
    private val tasks = mutableMapOf<String, Task>()
    private val worker = Executors.newSingleThreadExecutor()
    private val control = Executors.newSingleThreadExecutor()
    private val main = Handler(Looper.getMainLooper())
    private fun key(ticket: ContentTicket) = "${ticket.id}:${ticket.epoch}"

    override fun onStartJob(params: JobParameters): Boolean {
        val ticket = NativeContent.ticket(params.extras) ?: return false
        if (params.jobId != NativeContent.JOB_ID || Application.getProcessName() != "$packageName:downloads") return false
        val key = key(ticket)
        if (tasks.containsKey(key)) return false
        val task = Task(params, ticket); tasks[key] = task
        val notifications = getSystemService(NotificationManager::class.java)
        notifications.createNotificationChannel(NotificationChannel(CHANNEL, getString(R.string.content_download_channel), NotificationManager.IMPORTANCE_LOW))
        val open = PendingIntent.getActivity(this, 0, Intent(this, MainActivity::class.java).setAction(Intent.ACTION_MAIN)
            .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_CLEAR_TOP), PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE)
        val notification = Notification.Builder(this, CHANNEL).setSmallIcon(android.R.drawable.stat_sys_download)
            .setContentTitle(getString(R.string.content_download_title)).setContentText(getString(R.string.content_download_running))
            .setOngoing(true).setContentIntent(open).build()
        setNotification(params, NativeContent.JOB_ID, notification, JobService.JOB_END_NOTIFICATION_POLICY_REMOVE)
        worker.execute { run(task) }
        return true
    }

    private fun run(task: Task) {
        val connectivity = getSystemService(ConnectivityManager::class.java)
        var run: ContentRunHandle? = null
        var bound = false
        var unbound = true
        try {
            val owner = NativeContent.open(applicationContext)
            synchronized(task.lock) { task.owner = owner }
            // Claim first. An old OS callback cannot alter routing before Core rejects its ticket.
            run = contentClaim(owner, task.ticket)
            if (task.stopped.get()) contentRequestStop(owner, task.ticket, ContentControl.PAUSE)
            if (contentRunNeedsNetwork(run)) {
                val network = task.params.network
                val capabilities = network?.let(connectivity::getNetworkCapabilities)
                if (network == null || capabilities == null || !capabilities.hasTransport(NetworkCapabilities.TRANSPORT_WIFI) ||
                    !capabilities.hasCapability(NetworkCapabilities.NET_CAPABILITY_INTERNET) || !connectivity.bindProcessToNetwork(network)) {
                    contentRequestStop(owner, task.ticket, ContentControl.PAUSE)
                } else { bound = true }
            }
            if (bound) contentExecute(run) else contentExecuteStop(run)
        } catch (_: Exception) {
            // Core retains the exact failure/stop proof; no native success inference or automatic retry.
        } finally {
            if (bound) unbound = try { connectivity.bindProcessToNetwork(null) } catch (_: Exception) { false }
            if (run != null) {
                // Only confirm after execute and every socket/client has returned and routing is cleared.
                if (unbound) try { contentFinish(run) } catch (_: ContentException) { }
                run.destroy()
            }
            synchronized(task.lock) { task.owner?.destroy(); task.owner = null }
            main.post {
                val key = key(task.ticket)
                if (tasks[key] === task) tasks.remove(key)
                val pending = getSystemService(JobScheduler::class.java).getPendingJob(NativeContent.JOB_ID)
                if (pending != null && NativeContent.same(NativeContent.ticket(pending.extras), task.ticket)) jobFinished(task.params, false)
            }
        }
    }

    override fun onStopJob(params: JobParameters): Boolean {
        val ticket = NativeContent.ticket(params.extras) ?: return false
        val task = tasks[key(ticket)] ?: return false
        if (params.jobId != NativeContent.JOB_ID || !NativeContent.same(task.ticket, ticket)) return false
        task.stopped.set(true)
        control.execute { synchronized(task.lock) {
            task.owner?.let { owner -> try { contentRequestStop(owner, task.ticket, ContentControl.PAUSE) } catch (_: ContentException) { } }
        } }
        return false // Explicit in-app Resume; never automatically dispatch a new attempt.
    }

    override fun onDestroy() {
        tasks.values.forEach { task ->
            task.stopped.set(true)
            control.execute { synchronized(task.lock) {
                task.owner?.let { owner -> try { contentRequestStop(owner, task.ticket, ContentControl.PAUSE) } catch (_: ContentException) { } }
            } }
        }
        worker.shutdown(); control.shutdown()
        super.onDestroy()
    }
    companion object { private const val CHANNEL = "content-downloads" }
}
