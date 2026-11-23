package org.tonalli.spike.web

import android.content.Context
import java.io.File
import java.io.IOException
import java.util.concurrent.atomic.AtomicBoolean
import java.util.UUID
import org.tonalli.spike.benchmark.BenchmarkDataset
import org.tonalli.spike.benchmark.WebBenchmarkRequest
import org.tonalli.spike.benchmark.WebSessionReceipt
import org.tonalli.spike.withPack
import org.tonalli.spike.ai.AiWebAttachment
import org.tonalli.spike.reader.ReaderCatalogStore
import org.tonalli.spike.reader.ReaderCatalogPolicy
import org.tonalli.spike.reader.ReaderIdentity
import org.tonalli.spike.reader.ReaderIndexes
import org.tonalli.spike.downloads.NativeContent
import org.tonalli.spike.saf.SafDocuments
import org.tonalli.spike.saf.SafPackSession
import uniffi.tonalli_core.LoopbackHandle
import uniffi.tonalli_core.LoopbackInfo
import uniffi.tonalli_core.serverInfo
import uniffi.tonalli_core.serverRegisterTiles
import uniffi.tonalli_core.serverRegisterZim
import uniffi.tonalli_core.serverStartLoopback
import uniffi.tonalli_core.serverStopLoopback
import uniffi.tonalli_core.zimIdentity

/** Created and closed only on the screen's single native worker. */
internal class LoopbackSession private constructor(private val handle: LoopbackHandle, private val context: Context,
    private val sessionId: String) : AutoCloseable {
    private val closed = AtomicBoolean(false)
    private val nativeLock = Any()
    private var ai: AiWebAttachment? = null
    var aiOffReason: String? = null
        private set
    var readerNotice: String? = null
        private set
    lateinit var info: LoopbackInfo
        private set

    /** Called on an independent canceller; the same lock fences generated handle disposal. */
    fun requestAiStop() = synchronized(nativeLock) {
        if (!closed.get()) AiWebAttachment.stop(handle)
    }

    override fun close() = synchronized(nativeLock) {
        if (closed.compareAndSet(false, true)) {
            try { AiWebAttachment.stop(handle) } finally {
                try { serverStopLoopback(handle) } finally {
                    try { ai?.close() } finally {
                        ai = null
                        try { handle.destroy() } finally { WebSessionReceipt.clear(context, sessionId) }
                    }
                }
            }
        }
    }

    companion object {
        fun start(context: Context, benchmark: WebBenchmarkRequest? = null, enableAi: Boolean = false,
                  reader: Boolean = false,
                  cancelled: () -> Boolean = { false }, onStarting: (LoopbackSession) -> Unit = {}): LoopbackSession {
            WebSessionReceipt.clear(context)
            val webRoot = installPackagedUi(context)
            val session = LoopbackSession(serverStartLoopback(File(context.filesDir, "core").absolutePath, webRoot.toString()),
                context, UUID.randomUUID().toString())
            onStarting(session)
            try {
                AiWebAttachment.configure(context, session.handle)
                if (benchmark != null) {
                    val input = BenchmarkDataset.load(context, benchmark.dataset)
                    if (input.manifestHash != benchmark.manifestHash) throw IOException("Benchmark input identity mismatch")
                    input.withPack(input.prepared(context)) { serverRegisterZim(session.handle, "probe", it) }
                }
                val external = context.getExternalFilesDir(null)
                if (external != null) {
                    val zim = File(external, "probe.zim")
                    if (!reader && benchmark == null && zim.exists()) withPack(zim) { serverRegisterZim(session.handle, "probe", it) }
                    for (country in listOf("mx", "us", "ca")) {
                        val file = File(external, "map-$country.pmtiles")
                        if (file.exists()) withPack(file) { serverRegisterTiles(session.handle, country, it) }
                    }
                }
                if (reader) {
                    // A catalog/storage refusal must not make already selected libraries unusable.
                    try { NativeContent.attach(context, session.handle) } catch (_: Exception) { }
                    try { org.tonalli.spike.downloads.NativeCopies.attach(context, session.handle) } catch (failure: Exception) {
                        android.util.Log.w("TonalliCopy", "Native copy setup refused: ${failure.javaClass.simpleName}: ${failure.message?.take(160)}")
                        session.readerNotice = context.getString(org.tonalli.spike.R.string.nearby_copy_foreground_only)
                    }
                    try { org.tonalli.spike.downloads.NativeMaps.attach(context, session.handle) } catch (_: Exception) { }
                    org.tonalli.spike.music.NativeMusic.attach(context, session.handle)
                    val entries = try { ReaderCatalogStore(context).load() } catch (_: Exception) {
                        session.readerNotice = context.getString(org.tonalli.spike.R.string.reader_saved_failed)
                        emptyList()
                    }
                    var unavailable = 0
                    val documents = SafDocuments(context.contentResolver)
                    for (entry in entries) {
                        if (cancelled()) throw IOException("Reader closed")
                        try {
                            SafPackSession.open(documents, entry.selection, cancelled).use {
                                val actual = zimIdentity(it.handle)
                                ReaderCatalogPolicy.requireIdentity(entry, ReaderIdentity(actual.uuid, actual.storedMd5))
                                serverRegisterZim(session.handle, entry.id, it.handle)
                                ReaderIndexes.attach(context, session.handle, entry, it, cancelled)
                            }
                        } catch (error: Exception) {
                            if (cancelled()) throw IOException("Reader closed", error)
                            unavailable++
                        }
                    }
                    if (unavailable > 0) session.readerNotice = context.getString(org.tonalli.spike.R.string.reader_saved_unavailable, unavailable)
                    try { org.tonalli.spike.index.NativePreparation.attach(context, session.handle) } catch (_: Exception) { }
                    org.tonalli.spike.lan.NativeSharing.attach(context, session.handle)
                    org.tonalli.spike.tv.NativeTv.attach(context, session.handle)
                }
                session.info = serverInfo(session.handle)
                if (enableAi) {
                    try {
                        val attachment = AiWebAttachment.start(context, session.handle, cancelled)
                        synchronized(session.nativeLock) { session.ai = attachment }
                        if (cancelled()) { session.requestAiStop(); throw IOException("Shared AI screen departed") }
                    } catch (_: Exception) {
                        session.aiOffReason = "AI off: verified installed Qwen is unavailable, this build has no AI, or current Core admission refused. Normal offline tools remain available."
                    }
                }
                val policy = LocalOriginPolicy(session.info.port.toInt())
                if (!policy.allowsNavigation(session.info.launchUrl, true)) throw IOException("Invalid native loopback launch URL")
                if (benchmark != null) WebSessionReceipt.write(context, session.sessionId, benchmark, webRoot.fileName.toString().removePrefix("ui-"))
                return session
            } catch (error: Throwable) {
                try { session.close() } catch (cleanup: Throwable) { error.addSuppressed(cleanup) }
                throw error
            }
        }
    }
}
