package org.tonalli.spike.ai

import java.io.IOException
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicInteger
import org.tonalli.spike.web.WorkerSession

fun embeddingPolicyTests() {
    fun refused(block: () -> Unit) {
        try { block(); error("Expected embedding refusal") } catch (_: IOException) { }
    }
    check(AiEmbeddingProbe.input("hello") == listOf("hello"))
    refused { AiEmbeddingProbe.input(" \n") }
    refused { AiEmbeddingProbe.input("a\u0000b") }
    refused { AiEmbeddingProbe.input("é".repeat(8193)) }
    check(AiEmbeddingProbe.input("é".repeat(8192)).single().toByteArray().size == 16384)
    val valid = AiEmbeddingProbe.assess(listOf(listOf(0.6f, 0.8f)), 2)
    check(valid.dimensions == 2 && kotlin.math.abs(valid.norm - 1.0) < 1e-3)
    refused { AiEmbeddingProbe.assess(emptyList(), 2) }
    refused { AiEmbeddingProbe.assess(listOf(listOf(1f), listOf(1f)), 1) }
    refused { AiEmbeddingProbe.assess(listOf(listOf(1f)), 2) }
    refused { AiEmbeddingProbe.assess(listOf(listOf(Float.NaN)), 1) }
    refused { AiEmbeddingProbe.assess(listOf(listOf(Float.POSITIVE_INFINITY)), 1) }
    refused { AiEmbeddingProbe.assess(listOf(listOf(0f)), 1) }
    refused { AiEmbeddingProbe.assess(listOf(listOf(2f)), 1) }
    refused { AiEmbeddingProbe.assess(listOf(listOf(1f)), 0) }

    val gate = AiRunGate().apply { resume() }
    val target = AiEmbeddingRun(gate::cancelled)
    val early = gate.begin()
    gate.cancel()
    check(target.cancellation() == null)
    check(!target.publish(early) { error("Early cancellation must prevent submission") })
    target.clear(early)
    check(target.cancellation() == null)

    val old = gate.begin()
    val oldClosed = AtomicBoolean(false)
    check(target.publish(old) { oldClosed.set(true) })
    val captured = checkNotNull(target.cancellation())
    target.clear(old)
    val next = gate.begin()
    val newClosed = AtomicBoolean(false)
    check(target.publish(next) { newClosed.set(true) })
    captured()
    check(oldClosed.get() && !newClosed.get())
    gate.depart()
    check(gate.cancelled(next))
    checkNotNull(target.cancellation())()
    check(newClosed.get())
    target.clear(next)

    // A real blocking close overlaps a returning embedding and worker release.
    val closeEntered = CountDownLatch(1)
    val releaseClose = CountDownLatch(1)
    val embeddingStarted = CountDownLatch(1)
    val embeddingAborted = CountDownLatch(1)
    val disposalAttempted = CountDownLatch(1)
    val disposed = AtomicInteger()
    val closes = AtomicInteger()
    val closure = AiModelClosure({
        closes.incrementAndGet()
        closeEntered.countDown()
        embeddingAborted.countDown()
        check(releaseClose.await(2, TimeUnit.SECONDS))
    }, { disposed.incrementAndGet() })
    val session = WorkerSession<AutoCloseable>()
    check(session.adopt(AutoCloseable { closure.dispose() }))
    val failure = java.util.concurrent.atomic.AtomicReference<Throwable?>()
    val worker = Thread {
        try {
            embeddingStarted.countDown()
            check(embeddingAborted.await(2, TimeUnit.SECONDS))
            disposalAttempted.countDown()
            session.release()
        } catch (error: Throwable) { failure.set(error) }
    }.apply { start() }
    check(embeddingStarted.await(2, TimeUnit.SECONDS))
    val canceller = Thread { try { closure.requestClose() } catch (error: Throwable) { failure.set(error) } }.apply { start() }
    check(closeEntered.await(2, TimeUnit.SECONDS))
    check(disposalAttempted.await(2, TimeUnit.SECONDS))
    check(disposed.get() == 0) { "Generated handle destroyed during native close" }
    releaseClose.countDown()
    worker.join(2000); canceller.join(2000)
    check(!worker.isAlive && !canceller.isAlive)
    failure.get()?.let { throw it }
    closure.requestClose(); closure.dispose(); session.release()
    check(closes.get() == 1 && disposed.get() == 1)
    println("PASS: embedding input/vector checks, exact cancellation target, early/departure fencing and blocking native-close/dispose ordering")
}

fun comparisonRequestTests() {
    fun refused(block: () -> Unit) {
        try { block(); error("Expected comparison request refusal") } catch (_: IOException) { }
    }
    check(AiProbeRequest.parse(null, null, null) == AiProbeRequest(32u, null))
    check(AiProbeRequest.parse(32, null, null).outputTokens == 32u)
    check(AiProbeRequest.parse(128, null, null).outputTokens == 128u)
    for (value in listOf(0, 31, 64, 129, -1, "128", 128L)) refused { AiProbeRequest.parse(value, null, null) }
    refused { AiProbeRequest.parse(null, "question", null) }
    refused { AiProbeRequest.parse(null, null, "passage") }
    refused { AiProbeRequest.parse(null, " ", "passage") }
    refused { AiProbeRequest.parse(null, "question", " ") }
    refused { AiProbeRequest.parse(null, "question", "bad\u0000text") }
    refused { AiProbeRequest.parse(128, "question", "é".repeat(32768)) }
    val passage = "Línea primera.\n  Segunda línea: ñ.\n"
    val question = "¿Qué dice la segunda línea?"
    val parsed = AiProbeRequest.parse(128, question, passage)
    check(parsed.outputTokens == 128u)
    check(parsed.prompt == AiProbeRequest.INSTRUCTION + "\n\nPassage:\n" + passage + "\n\nQuestion:\n" + question + "\n/no_think")
    println("PASS: comparison default32, explicit32/128, paired fixture validation and unchanged passage bytes")
}
