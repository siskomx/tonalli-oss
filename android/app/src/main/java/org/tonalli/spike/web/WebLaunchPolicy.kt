package org.tonalli.spike.web

/** Single-task Intent delivery needs a fresh worker owner for every explicit UI launch. */
object WebLaunchPolicy {
    const val OPEN_AI = "org.tonalli.spike.OPEN_SHARED_AI"
    const val OPEN_READER = "org.tonalli.spike.OPEN_READER"
    fun recreateForIntent(action: String?, benchmarkLoadAction: String): Boolean =
        action != benchmarkLoadAction && (action == null || action == OPEN_AI || action == OPEN_READER)

    fun finishOnPause(aiRequested: Boolean, replacing: Boolean, changingConfiguration: Boolean): Boolean =
        aiRequested && !replacing && !changingConfiguration
}
