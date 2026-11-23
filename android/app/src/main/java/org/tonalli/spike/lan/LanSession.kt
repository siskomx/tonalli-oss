package org.tonalli.spike.lan

import android.content.Context
import java.io.File
import org.tonalli.spike.web.installPackagedUi
import org.tonalli.spike.withPack
import uniffi.tonalli_core.*

/** Only installed packs and an explicitly chosen media snapshot reach the LAN owner. */
internal class LanSession private constructor(val handle: LanHandle) : AutoCloseable {
    override fun close() {
        try { serverStopLan(handle) } catch (primary: Throwable) {
            try { handle.destroy() } catch (cleanup: Throwable) { primary.addSuppressed(cleanup) }
            throw primary
        }
        handle.destroy()
    }

    companion object {
        fun fromLibrary(context: Context, address: String, request: NativeSharing.Request): LanSession =
            LanSession(serverStartLan(request.source, LanConfig(address, 0u, 8u, request.packs,
                installPackagedUi(context).toString(), File(context.filesDir, "lan-identity").absolutePath, request.media)))
        fun start(context: Context, address: String): LanSession {
            val root = installPackagedUi(context).toString()
            val source = serverStartLoopback(File(context.filesDir, "lan-bootstrap").absolutePath, root)
            return transferLan({
                val ids = mutableListOf<String>()
                val external = context.getExternalFilesDir(null)
                if (external != null) {
                    val zim = File(external, "probe.zim")
                    if (zim.isFile) {
                        withPack(zim) { serverRegisterZim(source, "probe", it) }
                        ids += "probe"
                    }
                    for (country in listOf("mx", "us", "ca")) {
                        val file = File(external, "map-$country.pmtiles")
                        if (file.isFile) {
                            withPack(file) { serverRegisterTiles(source, country, it) }
                            ids += country
                        }
                    }
                }
                LanSession(serverStartLan(source, LanConfig(address, 0u, 8u, ids, root,
                    File(context.filesDir, "lan-identity").absolutePath, null)))
            }, { serverStopLoopback(source) }, { source.destroy() })
        }
    }
}
