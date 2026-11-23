package org.tonalli.spike.index

import java.io.IOException

/** One parity prime, then warm native calls; result validation is outside each timer. */
object IndexQuerySeries {
    class Result(val query: IndexTransferSpec.Query) {
        var primingCalls = 0
        val samplesMs = mutableListOf<Double>()
        var completed = false
    }
    fun <T> run(results: List<Result>, cancelled: () -> Boolean, withSelection: (() -> Unit) -> Unit,
        clock: () -> Long, search: (IndexTransferSpec.Query) -> T, verify: (IndexTransferSpec.Query, T) -> Unit) {
        fun current() = withSelection { if (cancelled()) throw IOException("Index query series cancelled") }
        for (result in results) {
            if (result.primingCalls != 0 || result.samplesMs.isNotEmpty()) throw IOException("Index query series is single-use")
            current()
            result.primingCalls = 1
            verify(result.query, search(result.query))
            current()
            repeat(30) {
                current()
                val start = clock()
                val hits = search(result.query)
                val end = clock()
                if (end < start) throw IOException("Invalid index query clock")
                result.samplesMs.add((end - start) / 1_000_000.0)
                current()
                verify(result.query, hits)
                current()
            }
            result.completed = true
        }
    }
}
