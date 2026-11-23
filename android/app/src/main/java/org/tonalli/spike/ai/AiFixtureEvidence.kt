package org.tonalli.spike.ai

/** Pure bounded evidence only; it owns no model, filesystem object or UI. */
class AiFixtureEvidence(question: String, context: String, prompt: String, val outputCap: UInt,
                        private val maxChunks: Int = 1024, private val maxBytes: Int = 256 * 1024) {
    private fun hash(text: String) = java.security.MessageDigest.getInstance("SHA-256")
        .digest(text.toByteArray(Charsets.UTF_8)).joinToString("") { "%02x".format(it) }
    private val questionHash = hash(question)
    private val contextHash = hash(context)
    private val promptHash = hash(prompt)
    private val chunks = mutableListOf<Chunk>()
    private var bytes = 0
    private var omitted = 0L
    private var cancellation = false
    private var terminal: Terminal? = null
    private var started: Started? = null
    init { require(maxChunks in 1..1024 && maxBytes in 1..256 * 1024 && outputCap in setOf(32u, 128u)) }
    data class Chunk(val id: ULong, val receivedNs: Long, val text: String)
    data class Started(val id: ULong, val receivedNs: Long)
    data class Terminal(val reason: String, val errorPresent: Boolean, val cancelled: Boolean, val stats: Map<String, String>, val id: ULong?, val receivedNs: Long?)
    data class Snapshot(val questionSha256: String, val contextSha256: String, val promptSha256: String,
                        val outputCap: UInt, val chunks: List<Chunk>, val omittedChunks: Long,
                        val cancellationRequested: Boolean, val terminal: Terminal?, val comparisonEligible: Boolean,
                        val firstRawNs: Long?, val firstUsefulAnswerNs: Long? = null, val started: Started? = null)
    @Synchronized fun started(id: ULong, receivedNs: Long) { if (started == null) started = Started(id, receivedNs) else omitted++ }
    @Synchronized fun text(id: ULong, receivedNs: Long, text: String) {
        val length = text.toByteArray(Charsets.UTF_8).size
        if (terminal != null || receivedNs < 0 || chunks.size >= maxChunks || length > maxBytes - bytes) { omitted++; return }
        chunks.add(Chunk(id, receivedNs, text)); bytes += length
    }
    @Synchronized fun cancellationRequested() { cancellation = true }
    @Synchronized fun terminal(reason: String, errorPresent: Boolean, cancelled: Boolean, stats: Map<String, String>, id: ULong? = null, receivedNs: Long? = null): Boolean {
        if (terminal != null) return false
        terminal = Terminal(reason, errorPresent, cancelled, stats.toMap(), id, receivedNs)
        return true
    }
    @Synchronized fun snapshot(): Snapshot {
        val end = terminal
        return Snapshot(questionHash, contextHash, promptHash, outputCap, chunks.toList(), omitted, cancellation, end,
            end != null && !end.errorPresent && !end.cancelled && !cancellation && omitted == 0L,
            chunks.firstOrNull { it.text.isNotEmpty() }?.receivedNs, started = started)
    }
}
