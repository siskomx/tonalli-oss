package org.tonalli.spike.web

/** Single-task delivery must never reuse the closed owner from an earlier AI launch. */
fun webLaunchPolicyTests() {
    val ai = "org.tonalli.spike.OPEN_SHARED_AI"
    val load = "org.tonalli.spike.WEBVIEW_BENCHMARK_LOAD"
    for (previousAi in listOf(false, true)) {
        for (next in listOf(null, ai, "org.tonalli.spike.OPEN_READER")) {
            check(WebLaunchPolicy.recreateForIntent(next, load)) {
                "Every explicit launch needs a fresh owner; previousAi=$previousAi"
            }
        }
    }
    check(!WebLaunchPolicy.recreateForIntent(load, load))
    check(!WebLaunchPolicy.recreateForIntent("unknown", load))
    check(WebLaunchPolicy.finishOnPause(aiRequested = true, replacing = false, changingConfiguration = false))
    check(!WebLaunchPolicy.finishOnPause(aiRequested = false, replacing = false, changingConfiguration = false))
    check(!WebLaunchPolicy.finishOnPause(aiRequested = true, replacing = true, changingConfiguration = false))
    check(!WebLaunchPolicy.finishOnPause(aiRequested = true, replacing = false, changingConfiguration = true))
    println("PASS: repeated explicit normal/AI launches, benchmark delivery and paused-owner disposal")
}
