package org.tonalli.spike.downloads

import android.app.job.JobInfo
import android.app.job.JobScheduler
import android.content.ComponentName
import android.content.Context
import android.net.NetworkCapabilities
import android.net.NetworkRequest
import android.os.PersistableBundle
import java.io.File
import uniffi.tonalli_core.*

internal object NativeMaps {
    const val JOB_ID = 53402
    private val lock = Any()
    fun open(context: Context) = mapsOpen(File(context.filesDir, "core").absolutePath)
    fun ticket(extras: PersistableBundle): MapTicket? {
        val id = extras.getString("country") ?: return null
        val epoch = extras.getLong("epoch", 0)
        return if (id in listOf("mx", "us", "ca") && epoch > 0) MapTicket(id, epoch.toULong()) else null
    }
    fun attach(context: Context, server: LoopbackHandle) {
        val app = context.applicationContext
        val owner = open(app)
        try {
            synchronized(lock) {
                if (app.getSystemService(JobScheduler::class.java).getPendingJob(JOB_ID) == null) {
                    try { mapsReconcile(owner) } catch (_: PackException) { }
                }
            }
            serverAttachCountryMaps(server, owner, object : MapScheduler {
                override fun dispatch(ticket: MapTicket): Boolean = synchronized(lock) {
                    try {
                        val scheduler = app.getSystemService(JobScheduler::class.java)
                        if (!scheduler.canRunUserInitiatedJobs() || ticket.epoch > Long.MAX_VALUE.toULong()) return@synchronized false
                        val extras = PersistableBundle().apply { putString("country", ticket.id); putLong("epoch", ticket.epoch.toLong()) }
                        val network = NetworkRequest.Builder().addTransportType(NetworkCapabilities.TRANSPORT_WIFI)
                            .addCapability(NetworkCapabilities.NET_CAPABILITY_INTERNET).build()
                        val job = JobInfo.Builder(JOB_ID, ComponentName(app, MapDownloadService::class.java))
                            .setExtras(extras).setUserInitiated(true).setRequiredNetwork(network).build()
                        scheduler.schedule(job) == JobScheduler.RESULT_SUCCESS
                    } catch (_: Exception) { false }
                }
            })
        } finally { owner.destroy() }
    }
}
