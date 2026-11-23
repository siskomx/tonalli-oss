package org.tonalli.spike.benchmark

import java.io.IOException

data class BenchmarkRequest(val dataset: String, val manifestHash: String, val runId: String, val mode: String) {
    companion object {
        const val ACTION = "org.tonalli.spike.BENCHMARK"
        fun checked(dataset: String?, hash: String?, run: String?, mode: String?, count: Int?): BenchmarkRequest {
            if (dataset == null || !Regex("[a-z][a-z0-9_-]{0,31}").matches(dataset) ||
                hash == null || !Regex("[a-f0-9]{64}").matches(hash) ||
                run == null || !Regex("[A-Za-z0-9][A-Za-z0-9_-]{0,63}").matches(run) ||
                mode !in setOf("warm", "fresh-suggest", "fresh-article") ||
                (count != null && count != if (mode == "warm") 30 else 1)) throw IOException("Invalid benchmark request")
            return BenchmarkRequest(dataset, hash, run, mode!!)
        }
    }
}

data class WebBenchmarkRequest(val dataset: String, val manifestHash: String, val profile: String) {
    companion object {
        const val ACTION = "org.tonalli.spike.WEBVIEW_BENCHMARK"
        const val LOAD = "org.tonalli.spike.WEBVIEW_BENCHMARK_LOAD"
        fun checked(dataset: String?, hash: String?, profile: String?): WebBenchmarkRequest {
            if (dataset == null || !Regex("[a-z][a-z0-9_-]{0,31}").matches(dataset) ||
                hash == null || !Regex("[a-f0-9]{64}").matches(hash) ||
                profile == null || !Regex("m1-perf-[A-Za-z0-9][A-Za-z0-9_-]{0,63}").matches(profile)) {
                throw IOException("Invalid WebView benchmark request")
            }
            return WebBenchmarkRequest(dataset, hash, profile)
        }
    }
}

/** Main-thread handshake: a server-ready blank document is released at most once. */
class DeferredWebLaunch {
    private var ready = false
    private var requested = false
    private var launched = false
    fun ready() { ready = true }
    fun request() { requested = true }
    fun takeLaunch(): Boolean {
        if (!ready || !requested || launched) return false
        launched = true
        return true
    }
}
