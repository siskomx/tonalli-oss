package org.tonalli.spike.lan

internal fun <T : AutoCloseable> transferLan(start: () -> T,
    stopBootstrap: () -> Unit, destroyBootstrap: () -> Unit): T {
    var lan: T? = null
    var failure: Throwable? = null
    try { lan = start() } catch (error: Throwable) { failure = error }
    for (cleanup in listOf(stopBootstrap, destroyBootstrap)) {
        try { cleanup() } catch (error: Throwable) {
            if (failure == null) failure = error else failure.addSuppressed(error)
        }
    }
    failure?.let { primary ->
        try { lan?.close() } catch (cleanup: Throwable) { primary.addSuppressed(cleanup) }
        throw primary
    }
    return checkNotNull(lan)
}
