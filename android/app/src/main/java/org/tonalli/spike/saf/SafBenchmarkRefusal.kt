package org.tonalli.spike.saf

import java.io.IOException

class SafBenchmarkRefusal(val reason: Reason) : IOException() {
    enum class Reason {
        ISO_SELECTION,
        SELECTION_CHANGED,
    }
    val userMessage: String get() = when (reason) {
        Reason.ISO_SELECTION -> "SAF benchmarks accept standalone or split ZIMs. ISO selections are not supported here."
        Reason.SELECTION_CHANGED -> "SAF selection changed. Reselect the ZIM and bind a new manual receipt."
    }
}

object SafBenchmarkErrors {
    fun message(error: Throwable): String = if (error is SafBenchmarkRefusal) error.userMessage
        else "Benchmark/preparation failed: ${error.javaClass.simpleName}; no success is claimed."
}
