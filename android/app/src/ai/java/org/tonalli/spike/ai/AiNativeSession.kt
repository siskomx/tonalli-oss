package org.tonalli.spike.ai

import java.nio.file.Path
import uniffi.tonalli_core.AiHandle
import uniffi.tonalli_core.AiInfo
import uniffi.tonalli_core.AiKind
import uniffi.tonalli_core.TokenSink
import uniffi.tonalli_core.aiClose
import uniffi.tonalli_core.aiGenerate
import uniffi.tonalli_core.aiEmbed
import uniffi.tonalli_core.aiRerank
import uniffi.tonalli_core.RerankHit
import uniffi.tonalli_core.aiInfo
import uniffi.tonalli_core.aiLoad
import uniffi.tonalli_core.aiAnswer
import uniffi.tonalli_core.AnswerOutcome
import uniffi.tonalli_core.AnswerRequest
import uniffi.tonalli_core.IndexHandle
import uniffi.tonalli_core.PackHandle
import uniffi.tonalli_core.LoopbackHandle
import uniffi.tonalli_core.serverAttachAi

/** Only this opt-in source set references generated optional AI bindings. */
class AiNativeSession private constructor(private val handle: AiHandle, val info: AiInfo, val spec: AiModelSpec) : AutoCloseable {
    private val closure = AiModelClosure({ aiClose(handle) }, { handle.destroy() })
    fun generate(prompt: String, sink: TokenSink, maxTokens: UInt = 32u): ULong = aiGenerate(handle, prompt, maxTokens, sink)
    fun answer(index: IndexHandle, pack: PackHandle, request: AnswerRequest, sink: TokenSink): AnswerOutcome {
        if (spec.role != "generation") throw java.io.IOException("Cited answers require the verified generation model")
        return aiAnswer(index, pack, handle, null, request, sink)
    }
    fun embed(texts: List<String>): List<List<Float>> {
        if (spec.role != "embedding") throw java.io.IOException("Load the verified embedding model first")
        return aiEmbed(handle, texts)
    }
    fun requestEmbeddingCancel() = closure.requestClose()
    fun attachToLoopback(server: LoopbackHandle) {
        if (spec.role != "generation") throw java.io.IOException("Shared chat requires a generation model")
        serverAttachAi(server, spec.id, handle)
    }
    fun rerank(query: String, documents: List<String>): List<RerankHit> {
        if (spec.role != "embedding") throw java.io.IOException("Load the verified embedding model first")
        return aiRerank(handle, query, documents)
    }
    override fun close() = closure.dispose()
    companion object {
        fun load(path: Path, spec: AiModelSpec): AiNativeSession {
            val handle = aiLoad(path.toString(), if (spec.role == "generation") AiKind.GENERATION else AiKind.EMBEDDING)
            try { return AiNativeSession(handle, aiInfo(handle), spec) } catch (error: Exception) {
                try { aiClose(handle) } catch (cleanup: Exception) { if (cleanup !== error) error.addSuppressed(cleanup) }
                try { handle.destroy() } catch (cleanup: Exception) { if (cleanup !== error) error.addSuppressed(cleanup) }
                throw error
            }
        }
    }
}
