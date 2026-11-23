package org.tonalli.spike.ai

import java.io.IOException
import java.security.MessageDigest

/** Trusted fixture boundaries and repeat accounting; cosine/order remain entirely in Core. */
object AiRerankInput {
    const val QUERY_PREFIX = "Represent this sentence for searching relevant passages: "
    data class ModelPin(val id: String, val bytes: Long, val sha256: String, val role: String = "embedding")
    data class QuestionPin(val id: String, val bytes: Int, val sha256: String)
    data class Authority(val version: Int, val kind: String, val catalogSha256: String,
                         val packSha256: String, val indexSha256: String, val questionsSha256: String,
                         val model: ModelPin, val queryPrefix: String, val questions: List<QuestionPin>)
    data class Passage(val rank: Int, val url: String, val text: String, val sha256: String)
    data class Decoded(val version: Int, val id: String, val questionEn: String, val retrievalQueryEn: String,
                       val packSha256: String, val indexSha256: String, val language: String,
                       val spanishRetrievalEvaluated: Boolean, val passages: List<Passage>)
    data class Verified(val fixtureId: String, val rawContextSha256: String, val query: String, val documents: List<String>)
    data class Ranking(val indices: List<UInt>, val scores: List<Double>)
    data class Measurement(val repeat: Int, val durationNs: Long, val ranking: Ranking)

    private fun fail(message: String): Nothing = throw IOException("Rerank fixture: $message")
    private fun hash(bytes: ByteArray) = MessageDigest.getInstance("SHA-256").digest(bytes).joinToString("") { "%02x".format(it) }
    private fun hex(value: String) = value.matches(Regex("[a-f0-9]{64}"))
    private fun authority(value: Authority) {
        if (value.version != 1 || value.kind != "tonalli-rerank-inputs" || value.queryPrefix != QUERY_PREFIX ||
            listOf(value.catalogSha256, value.packSha256, value.indexSha256, value.questionsSha256, value.model.sha256).any { !hex(it) } ||
            value.model.id != "bge-small-en-v1.5-q8-0" || value.model.bytes <= 0 || value.model.role != "embedding" ||
            value.questions.size != 10 || value.questions.map { it.id }.toSet() != (1..10).map { "q${it.toString().padStart(2, '0')}" }.toSet() ||
            value.questions.any { it.bytes !in 1..65536 || !hex(it.sha256) }) fail("invalid trusted asset")
    }
    fun verifyRaw(authority: Authority, rawContext: ByteArray): QuestionPin {
        authority(authority)
        if (rawContext.size !in 1..65536) fail("raw question.json exceeds 64 KiB")
        val digest = hash(rawContext)
        return authority.questions.singleOrNull { it.bytes == rawContext.size && it.sha256 == digest }
            ?: fail("raw question.json bytes differ from trusted catalog")
    }
    fun verifyParsed(authority: Authority, pin: QuestionPin, query: String, decoded: Decoded, loaded: ModelPin): Verified {
        authority(authority)
        if (pin !in authority.questions || decoded.version != 1 || decoded.id != pin.id ||
            decoded.packSha256 != authority.packSha256 || decoded.indexSha256 != authority.indexSha256 ||
            decoded.language != "en" || decoded.spanishRetrievalEvaluated || loaded != authority.model ||
            decoded.questionEn.isBlank() || '\u0000' in decoded.questionEn || decoded.questionEn.toByteArray(Charsets.UTF_8).size > 2048 ||
            decoded.retrievalQueryEn != decoded.questionEn || query != QUERY_PREFIX + decoded.questionEn)
            fail("query, dataset or loaded embedding model differs from trusted fixture")
        if (decoded.passages.size != 20 || decoded.passages.map { it.rank } != (1..20).toList()) fail("expected twenty original BM25 ranks")
        for (passage in decoded.passages) {
            val bytes = passage.text.toByteArray(Charsets.UTF_8)
            if (passage.url.isBlank() || passage.url.length > 2048 || '\u0000' in passage.url ||
                passage.text.isBlank() || '\u0000' in passage.text || bytes.size !in 1..1024 ||
                !hex(passage.sha256) || hash(bytes) != passage.sha256) fail("source passage identity, bytes or checksum differs")
        }
        return Verified(pin.id, pin.sha256, query, decoded.passages.map { it.text })
    }
    fun ranking(indices: List<UInt>, scores: List<Double>): Ranking {
        if (indices.size != 20 || scores.size != 20 || indices.toSet().size != 20 || indices.any { it >= 20u } ||
            scores.any { !it.isFinite() || it !in -1.0..1.0 }) fail("invalid owned Core permutation or score")
        return Ranking(indices.toList(), scores.toList())
    }
    fun series(cancelled: () -> Boolean, clock: () -> Long, call: () -> Ranking,
               completed: (Measurement) -> Unit) {
        for (repeat in 1..3) {
            if (cancelled()) fail("cancelled before Core call")
            val start = clock()
            val result = call()
            val end = clock()
            if (start < 0 || end < start) fail("invalid monotonic call duration")
            completed(Measurement(repeat, end - start, ranking(result.indices, result.scores)))
        }
    }
}
