package org.tonalli.spike.downloads

import android.content.Context
import android.content.Intent
import java.io.File
import org.tonalli.spike.lan.LanBinding
import org.tonalli.spike.lan.LanNetwork
import uniffi.tonalli_core.*

/** Process-owned copy state only; never retains the reader or persists pairing credentials. */
internal object NativeCopies {
    private val lock = Any()
    private var owner: NearbyCopyHandle? = null
    private var pending: Pair<ULong, LanBinding>? = null

    fun attach(context: Context, server: LoopbackHandle) = synchronized(lock) {
        val app = context.applicationContext
        val handle = owner ?: nearbyCopyOpen(File(app.filesDir, "core").absolutePath, object : NearbyCopyScheduler {
            override fun dispatch(epoch: ULong, ipv6: Boolean): String = synchronized(lock) {
                if (epoch == 0uL || epoch > Long.MAX_VALUE.toULong()) throw PackException.Mismatch("Copy attempt unavailable")
                val binding = LanNetwork(app).wifiBinding(ipv4Only = !ipv6, ipv6Only = ipv6)
                    ?: throw PackException.Mismatch("Connect to Wi-Fi before copying")
                pending = epoch to binding
                try {
                    app.startForegroundService(Intent(app, NearbyCopyService::class.java)
                        .setAction(NearbyCopyService.START).putExtra(NearbyCopyService.EPOCH, epoch.toLong()))
                } catch (failure: RuntimeException) {
                    if (pending?.first == epoch) pending = null
                    throw PackException.Mismatch("Android refused the background copy")
                }
                binding.address
            }
        }).also { owner = it }
        serverAttachNearbyCopies(server, handle)
    }
    fun take(epoch: ULong): LanBinding? = synchronized(lock) {
        pending?.takeIf { it.first == epoch }?.also { pending = null }?.second
    }
    fun state(): CopyRunState? = synchronized(lock) { owner?.let(::nearbyCopyState) }
    fun pause(epoch: ULong) = synchronized(lock) {
        if (epoch != 0uL) owner?.let { runCatching { nearbyCopyPause(it, epoch) } }
        Unit
    }
}
