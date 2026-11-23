package org.tonalli.spike.ai

/** Bounded foreground measurement policy; native work stays with the existing worker/session. */
class AiMeasurementRun(val spec: AiModelSpec, private val gate: AiRunGate) {
    data class Sample(val scheduledUs: Long, val observedUs: Long)
    data class Failure(val errorClass: String, val reason: String?, val stage: String)
    companion object {
        private fun bounded(value: String, limit: Int): String =
            value.take(limit).map { if (it.isISOControl()) ' ' else it }.joinToString("")
        fun failure(error: Throwable, stage: String): Failure = Failure(
            bounded(error.javaClass.simpleName, 64), error.message?.let { bounded(it, 256) }, bounded(stage, 64))
    }
    private val calibrationInputs = mutableSetOf<Int>()
    private var stopped = false
    private var previous: Long? = null
    private var requests = 0
    private var lastSampleUs = 0L
    private var nextSampleUs = 30_000_000L

    fun nextCalibration(repeats: Int, observedInputTokens: UInt): Int? {
        require(repeats in 1..2000 && observedInputTokens in 1u..2048u)
        require(calibrationInputs.size < 16 && calibrationInputs.add(repeats))
        if (observedInputTokens == 2000u) return null
        require(calibrationInputs.size < 16)
        return (repeats + 2000 - observedInputTokens.toInt()).also { require(it in 1..2000) }
    }

    @Synchronized fun beginRequest(): Long? {
        if (stopped || previous?.let { gate.cancelled(it) } == true) return null
        require(requests < 4096)
        val stamp = try { gate.begin() } catch (_: IllegalStateException) { stopped = true; return null }
        requests++
        previous = stamp
        return stamp
    }

    @Synchronized fun cancel(): ULong? { stopped = true; return previous?.let { gate.cancel(it) } }

    @Synchronized fun shouldContinue(completedRequestWallUs: Long): Boolean {
        require(completedRequestWallUs >= 0)
        return !stopped && completedRequestWallUs < 600_000_000L && previous?.let { gate.cancelled(it) } != true
    }

    fun sampleDue(elapsedUs: Long): Sample? {
        require(elapsedUs >= lastSampleUs)
        lastSampleUs = elapsedUs
        if (elapsedUs < nextSampleUs) return null
        val sample = Sample(nextSampleUs, elapsedUs)
        // Skip missed slots; a current sensor reading cannot reconstruct the past.
        nextSampleUs = Math.multiplyExact(Math.addExact(elapsedUs / 30_000_000L, 1L), 30_000_000L)
        return sample
    }
}
