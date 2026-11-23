package org.tonalli.spike.index

import android.content.Context
import android.content.Intent
import android.net.Uri
import java.io.File
import java.util.concurrent.Executors
import java.util.concurrent.atomic.AtomicBoolean
import org.tonalli.spike.saf.SafPackSession
import org.tonalli.spike.web.WorkerSession
import uniffi.tonalli_core.*

/** Only exact Core tickets reach the independent native owner, never web paths or descriptors. */
internal object NativePreparation {
    class Task(val ticket: PreparationTicket) {
        val lock = Any()
        val stopped = AtomicBoolean(false)
        val source = WorkerSession<SafPackSession>()
        var owner: PreparationHandle? = null
        var service: SearchPreparationService? = null
        var wake: android.os.PowerManager.WakeLock? = null
        fun stop() { stopped.set(true); source.requestClose() }
    }
    private val lock = Any()
    private val tasks = mutableMapOf<String, Task>()
    // The same native executor also fences a predecessor's post-publication cleanup.
    val worker = Executors.newSingleThreadExecutor()
    private val controls = Executors.newSingleThreadExecutor()
    private fun key(ticket: PreparationTicket) = "${ticket.pack}:${ticket.epoch}"
    private fun same(a: PreparationTicket?, b: PreparationTicket) = a != null && a.pack == b.pack && a.epoch == b.epoch
    fun open(context: Context) = preparationOpen(File(context.filesDir, "core").absolutePath)
    fun ticket(intent: Intent?): PreparationTicket? {
        val (pack, epoch) = preparationIdentity(intent?.getStringExtra("pack"), intent?.getStringExtra("epoch")) ?: return null
        return PreparationTicket(pack, epoch)
    }
    fun intent(context: Context, ticket: PreparationTicket, action: String) =
        Intent(context, SearchPreparationService::class.java).setAction(action)
            .setData(Uri.parse("tonalli-preparation:${ticket.epoch}"))
            .putExtra("pack", ticket.pack).putExtra("epoch", ticket.epoch.toString())

    fun attach(context: Context, server: LoopbackHandle) {
        val app = context.applicationContext
        val owner = open(app)
        try {
            synchronized(lock) {
                val active = preparationActiveTicket(owner)
                if (active != null && tasks[key(active)] == null) {
                    // This main-process service has no accepted/running ticket; Core also checks its lease.
                    preparationReconcile(owner, active)
                }
            }
            serverAttachPreparation(server, owner, object : PreparationScheduler {
                override fun dispatch(owner: PreparationHandle, ticket: PreparationTicket): ContentDispatch =
                    try { schedule(app, owner, ticket) } catch (_: Exception) { ContentDispatch.UNCERTAIN } finally { owner.destroy() }
                override fun control(owner: PreparationHandle, ticket: PreparationTicket, action: PreparationControl): ContentDispatch {
                    return try {
                        synchronized(lock) {
                            val task = tasks[key(ticket)]
                            if (task != null) { task.stop(); ContentDispatch.ACCEPTED }
                            // Core may finish the durable stop before this callback acquires the owner fence.
                            else if (!same(preparationActiveTicket(owner), ticket)) ContentDispatch.ACCEPTED
                            else schedule(app, owner, ticket)
                        }
                    } catch (_: Exception) { ContentDispatch.UNCERTAIN } finally { owner.destroy() }
                }
            })
        } finally { owner.destroy() }
    }

    private fun schedule(context: Context, owner: PreparationHandle, ticket: PreparationTicket): ContentDispatch = synchronized(lock) {
        if (!same(preparationActiveTicket(owner), ticket)) return@synchronized ContentDispatch.REJECTED
        if (tasks.containsKey(key(ticket))) return@synchronized ContentDispatch.ACCEPTED
        val task = Task(ticket)
        tasks[key(ticket)] = task
        try {
            if (context.startForegroundService(intent(context, ticket, SearchPreparationService.START)) == null) {
                tasks.remove(key(ticket)); ContentDispatch.REJECTED
            } else ContentDispatch.ACCEPTED
        } catch (_: IllegalStateException) {
            tasks.remove(key(ticket)); ContentDispatch.REJECTED
        } catch (_: SecurityException) {
            tasks.remove(key(ticket)); ContentDispatch.REJECTED
        } catch (_: Exception) {
            // An unconfirmed OS admission must not be relabelled rejected or automatically retried.
            ContentDispatch.UNCERTAIN
        }
    }
    fun take(ticket: PreparationTicket, service: SearchPreparationService): Task? = synchronized(lock) {
        tasks[key(ticket)]?.takeIf { it.service == null }?.also { it.service = service }
    }
    fun complete(task: Task, finish: () -> Unit) = synchronized(lock) {
        // A same-epoch Cancel after Paused must not be swallowed by the predecessor's tail.
        try { finish() } finally { if (tasks[key(task.ticket)] === task) tasks.remove(key(task.ticket)) }
    }
    fun requestPause(task: Task) {
        task.stop()
        controls.execute { synchronized(task.lock) {
            task.owner?.let { owner -> runCatching { preparationRequestStop(owner, task.ticket, PreparationControl.PAUSE) } }
        } }
    }
    fun notificationControl(context: Context, ticket: PreparationTicket, action: PreparationControl) {
        val app = context.applicationContext
        controls.execute {
            val task = synchronized(lock) { tasks[key(ticket)] } ?: return@execute
            val owner = try { open(app) } catch (_: PreparationException) { return@execute }
            try { preparationRequestStop(owner, ticket, action); task.stop() }
            catch (_: PreparationException) { /* A stale notification cannot control a new epoch. */ }
            finally { owner.destroy() }
        }
    }
}
