package org.tonalli.spike.ai

fun measurementPolicyTests() {
    calibrationRequiresActualRenderedCount()
    calibrationIsBounded()
    seriesCancellationSurvivesRequestBoundary()
    pendingCancellationUsesExistingGate()
    oldSeriesCannotCancelNewEpoch()
    measurementDurationAndSamplingAreHonest()
    failureDiagnosticPreservesBoundedReasonAndStage()
}

private fun failureDiagnosticPreservesBoundedReasonAndStage() {
    val timeout = AiMeasurementRun.failure(java.io.IOException("Measurement request deadline"), "calibration_2_wait")
    check(timeout.errorClass == "IOException")
    check(timeout.reason == "Measurement request deadline")
    check(timeout.stage == "calibration_2_wait")
    val bounded = AiMeasurementRun.failure(java.io.IOException("native error code 7\n\r\t" + "x".repeat(2000)), "s".repeat(200))
    check(bounded.reason?.startsWith("native error code 7") == true)
    val reason = checkNotNull(bounded.reason)
    check(reason.length <= 256 && reason.none { it.isISOControl() })
    check(bounded.stage.length <= 64 && bounded.errorClass.length <= 64)
    check(AiMeasurementRun.failure(java.io.IOException(), "submit").reason == null)
}

private fun oldSeriesCannotCancelNewEpoch() {
    val (old, gate) = measurement()
    val stamp = checkNotNull(old.beginRequest())
    gate.started(stamp, 19uL)
    check(old.cancel() == 19uL)

    val (departed, sharedGate) = measurement()
    checkNotNull(departed.beginRequest())
    sharedGate.depart()
    sharedGate.resume()
    val fresh = sharedGate.begin()
    check(sharedGate.started(fresh, 20uL).accepted)
    check(departed.cancel() == null)
    check(!sharedGate.cancelled(fresh))
    check(sharedGate.text(fresh, 20uL))
}

private fun measurement(): Pair<AiMeasurementRun, AiRunGate> {
    val gate = AiRunGate().apply { resume() }
    val spec = AiModelSpec.checked("held-model", "generation", 123L, "a".repeat(64))
    return AiMeasurementRun(spec, gate) to gate
}

private fun refuses(work: () -> Unit) {
    check(runCatching(work).exceptionOrNull() is IllegalArgumentException)
}

private fun calibrationRequiresActualRenderedCount() {
    val (run, _) = measurement()
    check(run.nextCalibration(1000, 1012u) == 1988)
    check(run.nextCalibration(1988, 2000u) == null)
    check(run.spec.id == "held-model" && run.spec.bytes == 123L)
    val (overshoot, _) = measurement()
    check(overshoot.nextCalibration(2000, 2048u) == 1952)
    for ((repeats, observed) in listOf(0 to 2000u, 2001 to 2000u, 1000 to 0u,
            1000 to 2049u, 1 to 2048u, 2000 to 1999u)) {
        refuses { measurement().first.nextCalibration(repeats, observed) }
    }
}

private fun calibrationIsBounded() {
    val (repeated, _) = measurement()
    check(repeated.nextCalibration(1000, 1001u) == 1999)
    refuses { repeated.nextCalibration(1000, 1001u) }
    val (run, _) = measurement()
    repeat(15) { index -> check(run.nextCalibration(1000 + index, 1999u) == 1001 + index) }
    refuses { run.nextCalibration(1015, 1999u) }
    val (last, _) = measurement()
    repeat(15) { index -> last.nextCalibration(1000 + index, 1999u) }
    check(last.nextCalibration(1015, 2000u) == null)
}

private fun seriesCancellationSurvivesRequestBoundary() {
    val (run, gate) = measurement()
    val first = checkNotNull(run.beginRequest())
    check(gate.started(first, 11uL).accepted)
    check(gate.finished(first, 11uL))
    check(run.cancel() == null)
    check(run.beginRequest() == null) // AiRunGate.begin alone would reset cancellation.
    check(!run.shouldContinue(1L))
}

private fun pendingCancellationUsesExistingGate() {
    val (run, gate) = measurement()
    val stamp = checkNotNull(run.beginRequest())
    check(run.cancel() == null)
    check(gate.started(stamp, 12uL).cancelId == 12uL)
    check(gate.returned(stamp, 12uL) == 12uL)
    check(!gate.text(stamp, 12uL))
    check(run.beginRequest() == null)
    val (departing, departedGate) = measurement()
    val old = checkNotNull(departing.beginRequest())
    departedGate.depart()
    check(departing.beginRequest() == null)
    check(!departedGate.permits(old))
}

private fun measurementDurationAndSamplingAreHonest() {
    val (run, _) = measurement()
    check(run.shouldContinue(599_999_999L))
    check(!run.shouldContinue(600_000_000L))
    refuses { run.shouldContinue(-1L) }
    check(run.sampleDue(29_999_999L) == null)
    check(run.sampleDue(30_000_007L) == AiMeasurementRun.Sample(30_000_000L, 30_000_007L))
    check(run.sampleDue(30_000_007L) == null)
    check(run.sampleDue(95_000_000L) == AiMeasurementRun.Sample(60_000_000L, 95_000_000L))
    check(run.sampleDue(95_000_000L) == null) // No fabricated historical readings.
    check(run.sampleDue(120_000_000L) == AiMeasurementRun.Sample(120_000_000L, 120_000_000L))
    refuses { run.sampleDue(119_999_999L) }
}
