package org.tonalli.spike.ai

import org.tonalli.spike.web.WorkerSession

fun main() {
    earlyStartedAndPendingCancel()
    staleEventsAndExactlyOneTerminal()
    departureAndLateLoadCleanup()
    measurementPolicyTests()
    embeddingPolicyTests()
    comparisonRequestTests()
    fixtureEvidenceTests()
    rerankInputTests()
    answerCaseTests()
    println("PASS: AI early Started/return, pending direct cancel, epoch fencing, one terminal, departed late-load cleanup")
}

private fun earlyStartedAndPendingCancel() {
    val gate = AiRunGate()
    gate.resume()
    val epoch = gate.begin()
    check(gate.permits(epoch))
    check(gate.cancel() == null)
    check(gate.cancelled(epoch))
    val started = gate.started(epoch, 41uL)
    check(started.accepted && started.cancelId == 41uL)
    check(!gate.text(epoch, 41uL))
    check(gate.returned(epoch, 41uL) == 41uL)
    check(gate.finished(epoch, 41uL))
    check(!gate.finished(epoch, 41uL))
    check(gate.cancel() == null)

    val next = gate.begin()
    check(gate.returned(next, 42uL) == null)
    check(gate.started(next, 42uL).accepted)
    check(gate.text(next, 42uL))
    check(gate.cancel() == 42uL)
    check(gate.finished(next, 42uL))
}

private fun staleEventsAndExactlyOneTerminal() {
    val gate = AiRunGate()
    gate.resume()
    val old = gate.begin()
    val current = gate.begin()
    val stale = gate.started(old, 10uL)
    check(!stale.accepted && stale.cancelId == 10uL)
    check(gate.returned(old, 10uL) == 10uL)
    check(!gate.text(old, 10uL) && !gate.finished(old, 10uL))
    check(gate.started(current, 11uL).accepted)
    check(!gate.text(current, 12uL) && !gate.finished(current, 12uL))
    check(gate.text(current, 11uL))
    check(gate.finished(current, 11uL))
    check(!gate.text(current, 11uL) && !gate.finished(current, 11uL))
    check(!gate.started(current, 11uL).accepted)
    // A very fast job can finish before aiGenerate returns its already-completed ID.
    check(gate.returned(current, 11uL) == null)
}

private fun departureAndLateLoadCleanup() {
    class Model : AutoCloseable {
        var closes = 0
        override fun close() { closes++ }
    }
    val gate = AiRunGate()
    val session = WorkerSession<Model>()
    gate.resume()
    val loading = gate.begin()
    check(gate.depart() == null)
    session.requestClose()
    val late = Model()
    check(!session.adopt(late) && late.closes == 1)
    check(!gate.permits(loading))
    check(!gate.started(loading, 91uL).accepted)
    check(gate.returned(loading, 91uL) == 91uL)
    check(!gate.text(loading, 91uL) && !gate.finished(loading, 91uL))
    session.release()
    check(late.closes == 1)

    gate.resume()
    check(!gate.permits(loading) && gate.cancelled(loading))
    val running = gate.begin()
    gate.started(running, 92uL)
    check(gate.depart() == 92uL)
    check(!gate.permits(running) && !gate.text(running, 92uL))
}
