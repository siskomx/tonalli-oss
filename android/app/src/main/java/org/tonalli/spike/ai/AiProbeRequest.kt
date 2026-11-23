package org.tonalli.spike.ai

data class AiProbeRequest(val outputTokens: UInt, val prompt: String?, val question: String? = null, val context: String? = null) {
    companion object {
        const val INSTRUCTION = "Answer only from the provided passage, in the question's language. Cite supporting numbered sources as [n]. If it does not contain the answer, say so. Keep the answer concise."
        fun parse(outputTokens: Any?, question: String?, context: String?): AiProbeRequest {
            val limit = when (outputTokens) {
                null -> 32u
                32 -> 32u
                128 -> 128u
                else -> throw java.io.IOException("output_tokens must be an explicit integer 32 or 128")
            }
            if (question == null && context == null) return AiProbeRequest(limit, null)
            if (question.isNullOrBlank() || context.isNullOrBlank() || '\u0000' in question || '\u0000' in context)
                throw java.io.IOException("Comparison requires paired nonblank question/context without NUL")
            val prompt = "$INSTRUCTION\n\nPassage:\n$context\n\nQuestion:\n$question\n/no_think"
            if (prompt.toByteArray(Charsets.UTF_8).size > 65536) throw java.io.IOException("Comparison prompt exceeds Core byte bound")
            return AiProbeRequest(limit, prompt, question, context)
        }
    }
}
