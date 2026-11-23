package org.tonalli.spike.ai

import java.io.IOException
import java.security.MessageDigest

fun rerankInputTests() {
    fun hash(value: String) = MessageDigest.getInstance("SHA-256").digest(value.toByteArray(Charsets.UTF_8)).joinToString("") { "%02x".format(it) }
    fun refused(block: () -> Unit) {
        try { block(); error("Expected rerank refusal") } catch (_: IOException) { }
    }
    val raw = "{\n  \"fixture\": \"raw ñ\"\n}".toByteArray(Charsets.UTF_8)
    val pins = (1..10).map { n ->
        val id = "q${n.toString().padStart(2, '0')}"
        if (n == 1) AiRerankInput.QuestionPin(id, raw.size, hash(raw.toString(Charsets.UTF_8)))
        else AiRerankInput.QuestionPin(id, id.length, hash(id))
    }
    val model = AiRerankInput.ModelPin("bge-small-en-v1.5-q8-0", 36685152L,
        "f046db1dc724cf4f6f0a0c5917e922823b73eb1d27b8f9a9c2797f7866974804")
    val authority = AiRerankInput.Authority(1, "tonalli-rerank-inputs", "a".repeat(64), "b".repeat(64),
        "c".repeat(64), "d".repeat(64), model, AiRerankInput.QUERY_PREFIX, pins)
    check(AiRerankInput.verifyRaw(authority, raw) == pins.first())
    refused { AiRerankInput.verifyRaw(authority, raw + byteArrayOf(10)) }
    refused { AiRerankInput.verifyRaw(authority, raw.copyOf().also { it[2] = 33 }) }
    refused { AiRerankInput.verifyRaw(authority, ByteArray(65537)) }
    refused { AiRerankInput.verifyRaw(authority.copy(version = 2), raw) }
    refused { AiRerankInput.verifyRaw(authority.copy(questions = pins.dropLast(1) + pins.first()), raw) }

    val question = "What makes the trefoil knot nontrivial?"
    val query = AiRerankInput.QUERY_PREFIX + question
    val passages = (1..20).map { n ->
        val text = "Source $n\nUnchanged ñ text."
        AiRerankInput.Passage(n, "C/Source_$n", text, hash(text))
    }
    val decoded = AiRerankInput.Decoded(1, "q01", question, question, authority.packSha256,
        authority.indexSha256, "en", false, passages)
    val verified = AiRerankInput.verifyParsed(authority, pins.first(), query, decoded, model)
    check(verified.query == query && verified.documents == passages.map { it.text })
    check(verified.rawContextSha256 == pins.first().sha256 && verified.fixtureId == "q01")
    refused { AiRerankInput.verifyParsed(authority, pins.first(), question, decoded, model) }
    refused { AiRerankInput.verifyParsed(authority, pins.first(), query, decoded.copy(id = "q02"), model) }
    refused { AiRerankInput.verifyParsed(authority, pins.first(), query, decoded.copy(packSha256 = "e".repeat(64)), model) }
    refused { AiRerankInput.verifyParsed(authority, pins.first(), query, decoded.copy(indexSha256 = "e".repeat(64)), model) }
    refused { AiRerankInput.verifyParsed(authority, pins.first(), query, decoded.copy(passages = passages.take(19)), model) }
    refused { AiRerankInput.verifyParsed(authority, pins.first(), query, decoded.copy(passages = passages.dropLast(1) + passages.first()), model) }
    refused { AiRerankInput.verifyParsed(authority, pins.first(), query, decoded.copy(passages = passages.toMutableList().also { it[0] = it[0].copy(text = "rewritten") }), model) }
    refused { AiRerankInput.verifyParsed(authority, pins.first(), query, decoded.copy(spanishRetrievalEvaluated = true), model) }
    refused { AiRerankInput.verifyParsed(authority, pins.first(), query, decoded, model.copy(role = "generation")) }
    refused { AiRerankInput.verifyParsed(authority, pins.first(), query, decoded, model.copy(sha256 = "0".repeat(64))) }

    val indices = (19 downTo 0).map { it.toUInt() }.toMutableList()
    val scores = MutableList(20) { 0.25 }
    val ranking = AiRerankInput.ranking(indices, scores)
    indices[0] = 0u; scores[0] = Double.NaN
    check(ranking.indices.first() == 19u && ranking.scores.first() == 0.25) { "Owned result snapshot changed" }
    check(ranking.indices == (19 downTo 0).map { it.toUInt() }) { "Kotlin must not reorder Core results" }
    refused { AiRerankInput.ranking(List(20) { 0u }, List(20) { 0.0 }) }
    refused { AiRerankInput.ranking((0..19).map { it.toUInt() }.dropLast(1) + 20u, List(20) { 0.0 }) }
    refused { AiRerankInput.ranking((0..18).map { it.toUInt() }, List(19) { 0.0 }) }
    for (bad in listOf(Double.NaN, Double.POSITIVE_INFINITY, 1.01, -1.01)) {
        refused { AiRerankInput.ranking((0..19).map { it.toUInt() }, List(20) { if (it == 0) bad else 0.0 }) }
    }
    var calls = 0
    var now = 0L
    val rows = mutableListOf<AiRerankInput.Measurement>()
    AiRerankInput.series({ false }, { now.also { now += 11 } }, { calls++; ranking }, rows::add)
    check(calls == 3 && rows.map { it.repeat } == listOf(1, 2, 3) && rows.all { it.durationNs == 11L })
    refused { AiRerankInput.series({ true }, { 0 }, { error("Early cancellation submitted a Core call") }, {}) }
    var stopped = false
    calls = 0
    rows.clear()
    refused { AiRerankInput.series({ stopped }, { now++ }, { calls++; stopped = true; ranking }, rows::add) }
    check(calls == 1 && rows.size == 1) { "Completed first call must remain recorded; no second submission after cancellation" }
    println("PASS: trusted raw fixture/pins/query/documents, owned Core permutation boundaries and exactly three timed rerank calls with cancellation")
}
