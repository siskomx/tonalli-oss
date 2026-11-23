package org.tonalli.spike.lan

/** All operations run on the service's single worker, including late callbacks. */
class LanLifecycle {
    private var epoch = 0L
    private val leases = mutableListOf<AutoCloseable>()
    var isStarting = false
        private set
    var isActive = false
        private set

    fun begin(lowBattery: Boolean): Long? {
        if (lowBattery || isStarting || isActive) return null
        epoch++
        isStarting = true
        return epoch
    }

    fun adopt(token: Long, lease: AutoCloseable): Boolean {
        if (token != epoch || (!isStarting && !isActive)) {
            lease.close()
            return false
        }
        leases += lease
        return true
    }

    fun complete(token: Long): Boolean {
        if (token != epoch || !isStarting) return false
        isStarting = false
        isActive = true
        return true
    }

    fun stop() {
        epoch++
        isStarting = false
        isActive = false
        val owned = leases.reversed()
        leases.clear()
        var failure: Throwable? = null
        for (lease in owned) {
            try { lease.close() } catch (error: Throwable) {
                if (failure == null) failure = error else failure.addSuppressed(error)
            }
        }
        failure?.let { throw it }
    }
}
