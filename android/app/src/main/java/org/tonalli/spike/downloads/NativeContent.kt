package org.tonalli.spike.downloads

import android.app.job.JobInfo
import android.app.job.JobScheduler
import android.content.ComponentName
import android.content.Context
import android.net.NetworkCapabilities
import android.net.NetworkRequest
import android.os.PersistableBundle
import java.io.File
import java.util.concurrent.Executors
import uniffi.tonalli_core.*

/** The normal WebView host schedules IDs only; all catalog/path authority stays in Core. */
internal object NativeContent {
    const val JOB_ID = 53401
    private const val ID = "content_id"
    private const val EPOCH = "content_epoch"
    private val scheduleLock = Any()
    private val controls = Executors.newSingleThreadExecutor()

    fun ticket(extras: PersistableBundle): ContentTicket? {
        val id = extras.getString(ID) ?: return null
        val epoch = extras.getLong(EPOCH, 0)
        return if (Regex("[A-Za-z0-9_-]{1,128}").matches(id) && epoch > 0) ContentTicket(id, epoch.toULong()) else null
    }
    fun same(a: ContentTicket?, b: ContentTicket?) = a != null && b != null && a.id == b.id && a.epoch == b.epoch
    fun open(context: Context) = contentOpen(File(context.filesDir, "core").absolutePath)
    private fun scheduler(context: Context) = context.getSystemService(JobScheduler::class.java)

    fun attach(context: Context, server: LoopbackHandle) {
        val app = context.applicationContext
        val owner = open(app)
        try {
            synchronized(scheduleLock) {
                val active = contentActiveTicket(owner)
                if (active != null && !same(ticket(scheduler(app).getPendingJob(JOB_ID)?.extras ?: PersistableBundle()), active)) {
                    try { contentReconcile(owner, active) } catch (_: ContentException) { /* Live owner or stale ticket remains authoritative. */ }
                    if (same(contentActiveTicket(owner), active)) stopQueued(open(app), active)
                }
            }
            serverAttachContentJobs(server, owner, object : ContentScheduler {
                override fun dispatch(owner: ContentHandle, ticket: ContentTicket): ContentDispatch {
                    try { return schedule(app, owner, ticket) } finally { owner.destroy() }
                }
                override fun control(owner: ContentHandle, ticket: ContentTicket, action: ContentControl): ContentDispatch {
                    return try {
                        synchronized(scheduleLock) {
                            val pending = scheduler(app).getPendingJob(JOB_ID)
                            if (pending != null && same(NativeContent.ticket(pending.extras), ticket)) scheduler(app).cancel(JOB_ID)
                            stopQueued(owner, ticket)
                        }
                        ContentDispatch.ACCEPTED
                    } catch (_: Exception) { owner.destroy(); ContentDispatch.UNCERTAIN }
                }
            })
        } finally { owner.destroy() }
    }

    private fun schedule(context: Context, owner: ContentHandle, ticket: ContentTicket): ContentDispatch = synchronized(scheduleLock) {
        try {
            if (ticket.epoch > Long.MAX_VALUE.toULong()) return@synchronized ContentDispatch.REJECTED
            if (!same(contentActiveTicket(owner), ticket)) return@synchronized ContentDispatch.REJECTED
            val scheduler = scheduler(context)
            val pending = scheduler.getPendingJob(JOB_ID)
            if (pending != null && same(NativeContent.ticket(pending.extras), ticket)) return@synchronized ContentDispatch.ACCEPTED
            if (!scheduler.canRunUserInitiatedJobs()) return@synchronized ContentDispatch.REJECTED
            // Scheduling replaces this app-owned fixed slot; old callbacks remain epoch-fenced.
            val extras = PersistableBundle().apply { putString(ID, ticket.id); putLong(EPOCH, ticket.epoch.toLong()) }
            val network = NetworkRequest.Builder().addTransportType(NetworkCapabilities.TRANSPORT_WIFI)
                .addCapability(NetworkCapabilities.NET_CAPABILITY_INTERNET).build()
            val job = JobInfo.Builder(JOB_ID, ComponentName(context, ContentDownloadService::class.java))
                .setExtras(extras).setUserInitiated(true).setRequiredNetwork(network).build()
            if (scheduler.schedule(job) == JobScheduler.RESULT_SUCCESS) ContentDispatch.ACCEPTED else ContentDispatch.REJECTED
        } catch (_: Exception) { ContentDispatch.UNCERTAIN }
    }

    private fun stopQueued(owner: ContentHandle, ticket: ContentTicket) {
        controls.execute {
            var run: ContentRunHandle? = null
            try {
                run = contentClaim(owner, ticket) // Busy/stale means another native owner must finish.
                try { contentExecuteStop(run) } finally { contentFinish(run) }
            } catch (_: ContentException) { /* Durable intent remains visible; never retry with a network here. */ }
            finally { run?.destroy(); owner.destroy() }
        }
    }
}
