package org.tonalli.spike.tv

import android.content.Context
import android.content.Intent
import android.os.Handler
import android.os.Looper
import java.util.UUID
import org.tonalli.spike.lan.LanNetwork
import org.tonalli.spike.R
import uniffi.tonalli_core.*

/** Only the validated content catalog is handed to the separate foreground owner. */
internal object NativeTv {
    data class Request(val id: String, val catalog: TvCatalog) : AutoCloseable {
        override fun close() = catalog.destroy()
    }
    private val lock = Any()
    private val main = Handler(Looper.getMainLooper())
    private var pending: Request? = null
    @Volatile var service: TvService? = null
    @Volatile var snapshot = TvStatus(false, false, null, emptyList(), null, emptyList())
    fun take(id: String): Request? = synchronized(lock) { pending?.takeIf { it.id == id }?.also { pending = null } }
    fun hasPending(): Boolean = synchronized(lock) { pending != null }
    fun cancelPending(id: String? = null): Boolean = synchronized(lock) {
        val request = pending?.takeIf { id == null || it.id == id } ?: return@synchronized false
        pending = null
        request.close()
        true
    }
    fun stopForExit() {
        val cancelled = cancelPending()
        val owner = service
        if (owner != null) owner.requestStop()
        else if (cancelled) snapshot = snapshot.copy(running = false, busy = false, url = null, error = null, selectedMedia = emptyList())
    }
    fun attach(context: Context, source: LoopbackHandle) {
        val app = context.applicationContext
        serverAttachTv(source, object : TvScheduler {
            override fun status(): TvStatus {
                val connected = runCatching { LanNetwork(app).wifiBinding(ipv4Only = true) != null }.getOrDefault(false)
                return snapshot.copy(networks = if (connected) listOf(SharingNetwork("wifi", "Wi-Fi")) else emptyList())
            }
            override fun start(network: String, catalog: TvCatalog) {
                synchronized(lock) {
                    if (network != "wifi" || pending != null || snapshot.running || snapshot.busy ||
                        runCatching { LanNetwork(app).wifiBinding(ipv4Only = true) }.getOrNull() == null) {
                        catalog.destroy()
                        throw PackException.Mismatch("Connect to Wi-Fi and choose media")
                    }
                    val request = Request(UUID.randomUUID().toString(), catalog)
                    val selection = try { catalog.selectedMedia() } catch (failure: Exception) { request.close(); throw failure }
                    pending = request
                    snapshot = snapshot.copy(busy = true, error = null, selectedMedia = selection)
                    main.post {
                        synchronized(lock) {
                            if (pending?.id == request.id) {
                                try { app.startForegroundService(Intent(app, TvService::class.java).setAction(TvService.START).putExtra("catalogRequest", request.id)) }
                                catch (failure: Exception) {
                                    android.util.Log.e("TonalliTv", "Foreground owner refused", failure)
                                    pending = null; request.close()
                                    snapshot = snapshot.copy(busy = false, error = app.getString(R.string.tv_start_failed), selectedMedia = emptyList())
                                }
                            }
                        }
                    }
                }
            }
            override fun stop() {
                cancelPending()
                service?.stopAndWait()
                snapshot = snapshot.copy(running = false, busy = false, url = null, error = null, selectedMedia = emptyList())
            }
            override fun detachView() {
                // A reclaimed private view must not stop the explicit foreground owner.
            }
        })
    }
}
