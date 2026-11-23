package org.tonalli.spike.index

import android.app.*
import android.content.Intent
import android.content.pm.ServiceInfo
import android.os.*
import java.io.IOException
import java.util.concurrent.TimeUnit
import org.tonalli.spike.MainActivity
import org.tonalli.spike.R
import org.tonalli.spike.reader.*
import org.tonalli.spike.saf.*
import uniffi.tonalli_core.*

/** User-started local processing, independent of the reader Activity; no automatic resume. */
class SearchPreparationService : Service() {
    private val main = Handler(Looper.getMainLooper())
    private val tasks = mutableSetOf<NativePreparation.Task>()
    private var latestStartId = 0
    private var destroyed = false
    private val renewWake = object : Runnable {
        override fun run() {
            if (destroyed) return
            tasks.forEach { task -> synchronized(task.lock) {
                if (!task.stopped.get()) task.wake?.acquire(TimeUnit.MINUTES.toMillis(30))
            } }
            main.postDelayed(this, TimeUnit.MINUTES.toMillis(25))
        }
    }
    override fun onBind(intent: Intent): IBinder? = null
    override fun onCreate() {
        super.onCreate()
        getSystemService(NotificationManager::class.java).createNotificationChannel(
            NotificationChannel(CHANNEL, getString(R.string.preparation_channel), NotificationManager.IMPORTANCE_LOW))
        main.postDelayed(renewWake, TimeUnit.MINUTES.toMillis(25))
    }
    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        latestStartId = startId
        val ticket = NativePreparation.ticket(intent)
        if (ticket == null) { stopIfIdle(); return START_NOT_STICKY }
        if (intent?.action == PAUSE || intent?.action == CANCEL) {
            NativePreparation.notificationControl(applicationContext, ticket,
                if (intent.action == CANCEL) PreparationControl.CANCEL else PreparationControl.PAUSE)
            stopIfIdle(); return START_NOT_STICKY
        }
        if (intent?.action != START) { stopIfIdle(); return START_NOT_STICKY }
        val task = NativePreparation.take(ticket, this) ?: run { stopIfIdle(); return START_NOT_STICKY }
        tasks.add(task)
        try {
            val open = PendingIntent.getActivity(this, 0, Intent(this, MainActivity::class.java)
                .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_CLEAR_TOP), PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
            fun action(name: String, label: Int) = Notification.Action.Builder(null, getString(label),
                PendingIntent.getService(this, 0, NativePreparation.intent(this, ticket, name), PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)).build()
            startForeground(NOTIFICATION, Notification.Builder(this, CHANNEL).setSmallIcon(android.R.drawable.stat_sys_download)
                .setContentTitle(getString(R.string.preparation_title)).setContentText(getString(R.string.preparation_running))
                .setContentIntent(open).setOngoing(true).addAction(action(PAUSE, R.string.preparation_pause))
                .addAction(action(CANCEL, R.string.preparation_cancel)).build(), ServiceInfo.FOREGROUND_SERVICE_TYPE_DATA_SYNC)
        } catch (_: RuntimeException) { task.stop(); stopSelf(startId) }
        NativePreparation.worker.execute { run(task) }
        return START_NOT_STICKY
    }

    private fun run(task: NativePreparation.Task) {
        var run: PreparationRunHandle? = null
        var cleanup = true
        try {
            val owner = NativePreparation.open(applicationContext)
            synchronized(task.lock) { task.owner = owner }
            val native = preparationReserve(owner, task.ticket)
            run = native
            if (task.stopped.get()) runCatching { preparationRequestStop(owner, task.ticket, PreparationControl.PAUSE) }
            if (!preparationRunControl(native)) {
                val wake = getSystemService(PowerManager::class.java).newWakeLock(PowerManager.PARTIAL_WAKE_LOCK, "tonalli:prepare-search")
                wake.setReferenceCounted(false)
                synchronized(task.lock) { task.wake = wake; wake.acquire(TimeUnit.MINUTES.toMillis(30)) }
                if (!preparationBindStored(native)) {
                    val store = ReaderCatalogStore(applicationContext)
                    val entry = store.load().firstOrNull { it.id == task.ticket.pack } ?: throw IOException("Selected book unavailable")
                    val pack = SafPackSession.open(SafDocuments(applicationContext.contentResolver), entry.selection, { task.stopped.get() })
                    if (!task.source.adopt(pack)) throw IOException("Preparation stopped")
                    val identity = zimIdentity(pack.handle)
                    try { ReaderCatalogPolicy.requireIdentity(entry, ReaderIdentity(identity.uuid, identity.storedMd5)) }
                    catch (failure: IOException) { preparationSourceFailed(native, PreparationSourceFailure.SOURCE_CHANGED); throw failure }
                    store.withCurrent(entry) { preparationBindPack(native, pack.handle) }
                }
                preparationExecute(native)
            }
        } catch (_: Exception) {
            run?.let { native ->
                if (task.stopped.get()) synchronized(task.lock) {
                    task.owner?.let { owner -> runCatching { preparationRequestStop(owner, task.ticket, PreparationControl.PAUSE) } }
                }
                val stopped = runCatching { preparationRunControl(native) }.getOrDefault(false)
                if (!stopped) runCatching { preparationSourceFailed(native, PreparationSourceFailure.UNAVAILABLE) }
            }
        } finally {
            try { task.source.release() } catch (_: Exception) { cleanup = false }
            try {
                NativePreparation.complete(task) {
                    try {
                        run?.let { native ->
                            val pendingCancel = cleanup && runCatching { preparationFinish(native) }.getOrDefault(false)
                            native.destroy()
                            if (pendingCancel) synchronized(task.lock) {
                                task.owner?.let { owner ->
                                    var stop: PreparationRunHandle? = null
                                    try {
                                        val owned = preparationReserve(owner, task.ticket)
                                        stop = owned
                                        if (preparationRunControl(owned)) preparationFinish(owned)
                                    } catch (_: PreparationException) { /* Retain truthful pending state. */ }
                                    finally { stop?.destroy() }
                                }
                            }
                        }
                    } finally {
                        synchronized(task.lock) {
                            try { task.wake?.let { if (it.isHeld) it.release() } }
                            finally { task.wake = null; task.owner?.destroy(); task.owner = null }
                        }
                    }
                }
            } finally { main.post { tasks.remove(task); stopIfIdle() } }
        }
    }
    private fun stopIfIdle() {
        if (tasks.isEmpty() && stopSelfResult(latestStartId)) stopForeground(STOP_FOREGROUND_REMOVE)
    }
    override fun onTaskRemoved(rootIntent: Intent?) {
        tasks.forEach(NativePreparation::requestPause)
        super.onTaskRemoved(rootIntent)
    }
    override fun onDestroy() {
        destroyed = true
        main.removeCallbacks(renewWake)
        tasks.forEach(NativePreparation::requestPause)
        super.onDestroy()
    }
    companion object {
        const val START = "org.tonalli.spike.PREPARE_SEARCH"
        private const val PAUSE = "org.tonalli.spike.PAUSE_SEARCH_PREPARATION"
        private const val CANCEL = "org.tonalli.spike.CANCEL_SEARCH_PREPARATION"
        private const val CHANNEL = "search-preparation"
        private const val NOTIFICATION = 12
    }
}
