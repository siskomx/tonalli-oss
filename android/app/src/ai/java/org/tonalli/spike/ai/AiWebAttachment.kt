package org.tonalli.spike.ai

import android.content.Context
import java.io.IOException
import org.tonalli.spike.web.WorkerSession
import uniffi.tonalli_core.LoopbackHandle
import uniffi.tonalli_core.serverDetachAi
import uniffi.tonalli_core.serverConfigureAiModelRoot

/** Owns a fresh model on the WebView worker, never the native probe's paused handle. */
internal class AiWebAttachment private constructor(private val model: WorkerSession<AiNativeSession>) : AutoCloseable {
    fun detach(server: LoopbackHandle) = stop(server)
    override fun close() = model.release()

    companion object {
        fun stop(server: LoopbackHandle) = serverDetachAi(server)
        fun configure(context: Context, server: LoopbackHandle) =
            serverConfigureAiModelRoot(server, context.filesDir.absolutePath)

        fun start(context: Context, server: LoopbackHandle, cancelled: () -> Boolean): AiWebAttachment {
            val owner = WorkerSession<AiNativeSession>()
            try {
                val spec = AiModelCatalog.model("qwen3-1.7b-q4-k-m")
                val path = AiModelFiles.installed(context.filesDir.toPath(), spec, cancelled)
                if (cancelled()) throw IOException("Shared AI screen departed")
                val loaded = AiNativeSession.load(path, spec) // Core reads actual RAM admission.
                owner.adopt(loaded)
                if (cancelled()) throw IOException("Shared AI screen departed")
                loaded.attachToLoopback(server)
                return AiWebAttachment(owner)
            } catch (error: Throwable) {
                try { owner.release() } catch (cleanup: Throwable) { if (cleanup !== error) error.addSuppressed(cleanup) }
                throw error
            }
        }
    }
}
