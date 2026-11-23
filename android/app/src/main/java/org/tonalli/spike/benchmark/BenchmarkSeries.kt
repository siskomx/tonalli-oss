package org.tonalli.spike.benchmark

import java.io.IOException
import kotlin.math.ceil

object BenchmarkSeries {
    class Samples {
        val suggest = mutableListOf<Double>()
        val article = mutableListOf<Double>()
        var primedSuggest = 0
        var primedArticle = 0
    }

    fun run(mode: String, samples: Samples, cancelled: () -> Boolean, clock: () -> Long,
        suggest: () -> Int, article: () -> Int) {
        fun checkActive() { if (cancelled()) throw IOException("Benchmark cancelled") }
        fun prime(call: () -> Int) { checkActive(); if (call() <= 0) throw IOException("Empty priming result"); checkActive() }
        fun measure(call: () -> Int, output: MutableList<Double>) {
            checkActive()
            val start = clock()
            val count = call()
            val elapsed = (clock() - start) / 1_000_000.0
            if (!elapsed.isFinite() || elapsed < 0) throw IOException("Invalid benchmark clock")
            output.add(elapsed)
            if (count <= 0) throw IOException("Empty benchmark result")
            checkActive()
        }
        when (mode) {
            "warm" -> {
                prime(suggest); samples.primedSuggest++
                prime(article); samples.primedArticle++
                repeat(30) { measure(suggest, samples.suggest); measure(article, samples.article) }
            }
            "fresh-suggest" -> measure(suggest, samples.suggest)
            "fresh-article" -> measure(article, samples.article)
            else -> throw IOException("Unknown benchmark mode")
        }
    }

    fun nearestRank(values: List<Double>, percentile: Double): Double {
        if (values.isEmpty() || values.any { !it.isFinite() || it < 0 } || !percentile.isFinite() || percentile <= 0 || percentile > 1) {
            throw IOException("Invalid percentile input")
        }
        return values.sorted()[ceil(values.size * percentile).toInt() - 1]
    }
}
