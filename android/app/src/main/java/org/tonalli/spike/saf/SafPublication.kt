package org.tonalli.spike.saf

/** Only bounded metadata publication belongs inside commit; resolve providers outside this gate. */
class SafPublication {
    private var epoch = 0L
    fun begin(): Long = synchronized(monitor) { ++epoch }
    fun invalidate() { synchronized(monitor) { ++epoch } }
    fun publish(expected: Long, commit: () -> Unit): Boolean = synchronized(monitor) {
        if (expected != epoch) false else { commit(); true }
    }
    companion object { private val monitor = Any() }
}
