package org.tonalli.spike.index

import java.io.IOException
import org.tonalli.spike.benchmark.BenchmarkSeries

fun checkIndexQuerySeries() {
    val queries = IndexTransferSpec.QUERY_NAMES.map { name -> IndexTransferSpec.Query(name,
        if (name == "photosynthesis") emptyList() else listOf(IndexTransferSpec.Hit("Known title", "C/Known"))) }
    val results = queries.map { IndexQuerySeries.Result(it) }
    val calls = mutableMapOf<String, Int>()
    var now = 0L
    IndexQuerySeries.run(results, { false }, { it() }, { now }, search = { query ->
        calls[query.query] = calls.getOrDefault(query.query, 0) + 1
        now += 2_000_000L
        query.hits
    }, verify = { query, hits -> IndexTransferSpec.requireHits(query, hits); now += 100_000_000L })
    check(calls.values.toList() == listOf(31, 31, 31))
    check(results.all { it.primingCalls == 1 && it.completed && it.samplesMs.size == 30 && it.samplesMs.all { sample -> sample == 2.0 } })
    check(BenchmarkSeries.nearestRank((1..30).map(Int::toDouble), .5) == 15.0)
    check(BenchmarkSeries.nearestRank((1..30).map(Int::toDouble), .95) == 29.0)
    var cancel = false
    var invoked = 0
    val cancelled = queries.map { IndexQuerySeries.Result(it) }
    try {
        IndexQuerySeries.run(cancelled, { cancel }, { it() }, { now }, search = { query ->
            if (++invoked == 2) cancel = true
            now += 1_000_000; query.hits
        }, verify = IndexTransferSpec::requireHits)
        error("Expected cancellation")
    } catch (_: IOException) { }
    check(invoked == 2 && cancelled.first().samplesMs.size == 1 && cancelled.none { it.completed })
    var current = true
    invoked = 0
    val stale = queries.map { IndexQuerySeries.Result(it) }
    try {
        IndexQuerySeries.run(stale, { false }, { work -> if (!current) throw IOException("Selection changed"); work() }, { now }, search = { query ->
            if (++invoked == 2) current = false
            now += 1_000_000; query.hits
        }, verify = IndexTransferSpec::requireHits)
        error("Expected selection fence")
    } catch (_: IOException) { }
    check(invoked == 2 && stale.first().samplesMs.size == 1 && stale.none { it.completed })
    val mismatch = queries.map { IndexQuerySeries.Result(it) }
    try {
        IndexQuerySeries.run(mismatch, { false }, { it() }, { now }, search = {
            listOf(IndexTransferSpec.Hit("Known title", "C/Wrong"))
        }, verify = IndexTransferSpec::requireHits)
        error("Expected priming parity rejection")
    } catch (_: IOException) { }
    check(mismatch.first().primingCalls == 1 && mismatch.all { it.samplesMs.isEmpty() && !it.completed })
    println("Index query one-prime/30-sample counts, validation excluded from timers, p50/p95, cancellation and selection fences passed")
}
