package org.tonalli.spike.lan

import android.content.Context
import android.content.Intent
import android.os.Handler
import android.os.Looper
import java.util.UUID
import uniffi.tonalli_core.*

/** Only a native-owned content handle crosses into the existing foreground owner. */
internal object NativeSharing {
    data class Request(val id: String, val source: LoopbackHandle, val packs: List<String>, val media: SharedMedia, val hotspot: Boolean = false) : AutoCloseable {
        override fun close() { try { source.destroy() } finally { media.destroy() } }
    }
    private val lock = Any()
    private val main = Handler(Looper.getMainLooper())
    private var pending: Request? = null
    private var window = java.lang.ref.WeakReference<android.view.Window>(null)
    fun bindWindow(value: android.view.Window) { window = java.lang.ref.WeakReference(value) }
    @Volatile var service: LanService? = null
    @Volatile var snapshot = SharingStatus(false, false, null, null, emptyList(), emptyList(), null, null, null, null, null)
        set(value) {
            field = value
            main.post {
                window.get()?.let {
                    if (value.running || value.busy) it.addFlags(android.view.WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
                    else it.clearFlags(android.view.WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
                }
            }
        }

    private fun hotspotAllowed(context: Context) =
        context.checkSelfPermission(android.Manifest.permission.NEARBY_WIFI_DEVICES) == android.content.pm.PackageManager.PERMISSION_GRANTED

    fun take(id: String): Request? = synchronized(lock) {
        pending?.takeIf { it.id == id }?.also { pending = null }
    }
    fun cancelPending() = synchronized(lock) { pending?.close(); pending = null }
    fun attach(context: Context, source: LoopbackHandle) {
        val app = context.applicationContext
        serverAttachSharing(source, object : SharingScheduler {
            override fun room(request: RoomRequest): RoomSnapshot {
                val owner=service
                if(owner!=null)return owner.room(request)
                if(request.action==RoomAction.GET||request.action==RoomAction.LEAVE)return RoomSnapshot(false,"","",false,0UL,emptyList(),emptyList())
                throw RoomException.Unavailable()
            }
            override fun status(): SharingStatus {
                val networks = buildList {
                    if (LanNetwork(app).wifiBinding() != null) add(SharingNetwork("wifi", "Wi-Fi"))
                    // The UI offers a permission link for the second id instead of a start button.
                    add(SharingNetwork(if (hotspotAllowed(app)) "hotspot" else "hotspot-permission", "Phone hotspot"))
                }
                return snapshot.copy(networks = networks)
            }
            override fun start(source: LoopbackHandle, network: String, packs: List<String>, media: SharedMedia) {
                synchronized(lock) {
                    val hotspot = network == "hotspot"
                    val reachable = if (hotspot) hotspotAllowed(app) else network == "wifi" && LanNetwork(app).wifiBinding() != null
                    if (!reachable || pending != null || snapshot.running || snapshot.busy) {
                        source.destroy()
                        media.destroy()
                        throw PackException.Mismatch("Connect to Wi-Fi or allow the phone hotspot, and choose installed content")
                    }
                    val request = Request(UUID.randomUUID().toString(), source, packs, media, hotspot)
                    pending = request
                    snapshot = snapshot.copy(busy = true, error = null)
                    main.post {
                        synchronized(lock) {
                            if (pending?.id == request.id) {
                                try { app.startForegroundService(Intent(app, LanService::class.java).setAction(LanService.START)
                                    .putExtra("libraryRequest", request.id).putExtra("hotspot", request.hotspot)) }
                                catch (_: Exception) { pending = null; request.close(); snapshot = snapshot.copy(busy = false, error = "Android refused sharing") }
                            }
                        }
                    }
                }
            }
            override fun stop() {
                synchronized(lock) { pending?.close(); pending = null }
                service?.stopAndWait()
                snapshot = snapshot.copy(running = false, busy = false, url = null, pairingCode = null, devices = emptyList(), certificateSha256 = null, nativeInvitation = null, hotspot = null)
            }
            override fun kick(id: String) { service?.disconnect(id) }
        })
    }
}
