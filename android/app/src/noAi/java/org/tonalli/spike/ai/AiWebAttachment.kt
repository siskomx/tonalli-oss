package org.tonalli.spike.ai

import android.content.Context
import java.io.IOException
import uniffi.tonalli_core.LoopbackHandle

/** Keeps ordinary non-AI builds independent from generated optional AI bindings. */
internal class AiWebAttachment private constructor() : AutoCloseable {
    fun detach(server: LoopbackHandle) { /* No optional attachment exists in this build. */ }
    override fun close() {}
    companion object {
        fun stop(server: LoopbackHandle) { /* No optional AI exists in this build. */ }
        fun configure(context: Context, server: LoopbackHandle) { /* No optional model controls. */ }
        fun start(context: Context, server: LoopbackHandle, cancelled: () -> Boolean): AiWebAttachment {
            throw IOException("Shared AI is not included in this build")
        }
    }
}
