package org.tonalli.spike.ai

import android.os.SystemClock
import java.io.IOException
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import uniffi.tonalli_core.GenerationEnd
import uniffi.tonalli_core.GenerationEvent
import uniffi.tonalli_core.PackException
import uniffi.tonalli_core.TokenSink
import uniffi.tonalli_core.aiCancel

/** Runs on the existing loader worker; callbacks only collect bounded stats and signal it. */
class AiMeasurementWorker(private val model: AiNativeSession, private val gate: AiRunGate,
                          private val cancelled: () -> Boolean, private val metrics: AiMeasurementMetrics) {
    private val policy = AiMeasurementRun(model.spec, gate)
    private var epochNs = 0L
    private var completedWallUs = 0L
    private var latestId: ULong? = null
    private var latestTps: Double? = null
    private var phase = "admission"
    private var stage = "admission"
    private fun nowUs() = (SystemClock.elapsedRealtimeNanos() - epochNs) / 1000
    private fun snapshot(observedUs: Long = nowUs()) { policy.sampleDue(observedUs)?.let { metrics.snapshot(it, latestId, latestTps) } }
    private fun stopCheck() {
        if (cancelled()) { policy.cancel()?.let { runCatching { aiCancel(it) } }; throw IOException("Measurement cancelled") }
    }
    private data class Completed(val event: GenerationEvent.Finished, val startUs: Long, val endUs: Long,
                                 val firstReceiptUs: Long?)

    private fun job(prompt: String, cap: UInt, measured: Boolean): Completed {
        stage = "$phase:prepare"
        stopCheck()
        val stamp = policy.beginRequest() ?: throw IOException("Measurement departed")
        val started = SystemClock.elapsedRealtimeNanos()
        val first = measured && epochNs == 0L
        if (first) epochNs = started
        val startUs = if (measured && !first) nowUs() else 0L
        val done = CountDownLatch(1)
        val lock = Any()
        var result: Completed? = null
        var failure: Exception? = null
        var firstReceipt: Long? = null
        var chunks = 0
        var bytes = 0
        val sink = object : TokenSink {
            override fun onEvent(event: GenerationEvent) {
                val receivedNs = SystemClock.elapsedRealtimeNanos()
                try {
                    when (event) {
                        is GenerationEvent.Started -> {
                            val accepted = gate.started(stamp, event.id)
                            val cancel = accepted.cancelId ?: if (cancelled()) event.id else null
                            cancel?.let { runCatching { aiCancel(it) } }
                        }
                        is GenerationEvent.Text -> if (gate.text(stamp, event.id)) synchronized(lock) {
                            val length = event.text.toByteArray().size
                            require(length in 1..1024 && chunks < 4096 && bytes + length <= 128 * 1024)
                            chunks++; bytes += length
                            if (firstReceipt == null) firstReceipt = (receivedNs - started) / 1000
                        }
                        is GenerationEvent.Finished -> if (gate.finished(stamp, event.id)) {
                            synchronized(lock) {
                                result = Completed(event, startUs, if (measured) nowUs() else (SystemClock.elapsedRealtimeNanos() - started) / 1000, firstReceipt)
                            }
                            done.countDown()
                        }
                    }
                } catch (error: Exception) { synchronized(lock) { failure = error }; done.countDown() }
            }
        }
        stage = "$phase:submit"
        val id = submit(prompt, sink, cap)
        (gate.returned(stamp, id) ?: if (cancelled()) id else null)?.let { runCatching { aiCancel(it) } }
        try {
            stage = "$phase:wait"
            while (!done.await(100, TimeUnit.MILLISECONDS)) {
                stopCheck()
                if (SystemClock.elapsedRealtimeNanos() - started >= 90_000_000_000L) throw IOException("Measurement request deadline")
                // The timestamp must precede completion if this row still names the previous job.
                val tick = if (measured) synchronized(lock) { if (result == null && failure == null) nowUs() else null } else null
                tick?.let { snapshot(it) }
            }
            return synchronized(lock) { failure?.let { throw it }; result ?: throw IOException("Missing terminal stats") }
        } catch (error: Exception) { runCatching { aiCancel(id) }; throw error }
    }

    private fun submit(prompt: String, sink: TokenSink, cap: UInt): ULong {
        val started = SystemClock.elapsedRealtimeNanos()
        while (true) {
            stopCheck()
            try { return model.generate(prompt, sink, cap) } catch (error: PackException.Mismatch) {
                if (error.v1 !in setOf("AI generation is busy", "AI callback dispatcher is busy", "AI delivery capacity exhausted") ||
                    SystemClock.elapsedRealtimeNanos() - started >= 250_000_000L) throw error
                Thread.sleep(2)
            }
        }
    }

    fun run(): String {
        try {
            require(model.spec.role == "generation")
            var repeats = 1000
            var prompt: String
            var probe = 0
            while (true) {
                phase = "calibration_${probe + 1}"
                stage = "$phase:prompt"
                prompt = "Explain this offline-library exercise clearly:\n" + " x".repeat(repeats) + "\n/no_think"
                require(prompt.toByteArray().size <= 16_384)
                val result = job(prompt, 1u, false)
                val event = result.event
                stage = "$phase:terminal"
                metrics.calibration(++probe, repeats, event.stats.inputTokens, event.reason.name, event.error != null,
                    result.endUs - result.startUs, event.stats.prefillUs, event.stats.firstTokenUs, result.firstReceiptUs)
                checkTerminal(event)
                stage = "$phase:token_count"
                repeats = policy.nextCalibration(repeats, event.stats.inputTokens) ?: break
            }
            var request = 0
            while (policy.shouldContinue(completedWallUs)) {
                phase = "request_${++request}"
                val result = job(prompt, 128u, true)
                val event = result.event
                val stats = event.stats
                val wall = result.endUs - result.startUs
                latestTps = if (wall > 0) stats.outputTokens.toDouble() * 1_000_000.0 / wall else null
                val phaseTps = if (stats.decodeUs > 0uL) stats.outputTokens.toDouble() * 1_000_000.0 / stats.decodeUs.toDouble() else null
                stage = "$phase:terminal"
                metrics.request(listOf(event.id, result.startUs, result.endUs, wall, stats.inputTokens, stats.outputTokens,
                    stats.setupUs, stats.prefillUs, stats.firstTokenUs, result.firstReceiptUs, stats.decodeUs, stats.totalUs,
                    latestTps, phaseTps, event.reason.name, event.error != null).joinToString(",") { it?.toString() ?: "" })
                latestId = event.id
                checkTerminal(event)
                stage = "$phase:stats"
                require(wall > 0 && stats.inputTokens == 2000u && stats.outputTokens in 1u..128u)
                completedWallUs = Math.addExact(completedWallUs, wall)
                snapshot()
            }
            stopCheck()
            stage = "complete:receipt"
            metrics.finish("complete", completedWallUs)
            return "Measurement recorded: ai-measurements/${metrics.directory.fileName}/run.json. Read raw CSV; completion is not a gate PASS."
        } catch (error: Exception) {
            metrics.finish(if (cancelled()) "cancelled" else "failed", completedWallUs, AiMeasurementRun.failure(error, stage))
            throw error
        }
    }

    private fun checkTerminal(event: GenerationEvent.Finished) {
        if (event.error != null || event.reason !in setOf(GenerationEnd.EOS, GenerationEnd.LIMIT)) {
            throw IOException("Native finished ${event.reason}: ${event.error?.take(256) ?: "no native error supplied"}")
        }
    }
}
