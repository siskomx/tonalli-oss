package org.tonalli.spike.web

import java.util.concurrent.atomic.AtomicBoolean

/** Adoption/release run on one worker; cancellation may come from the UI thread. */
class WorkerSession<T : AutoCloseable> {
    private val closing = AtomicBoolean(false)
    private var current: T? = null
    val isClosing: Boolean get() = closing.get()
    fun current(): T? = current

    fun requestClose() { closing.set(true) }

    fun adopt(resource: T): Boolean {
        if (current != null) {
            resource.close()
            error("A worker session is already owned")
        }
        if (closing.get()) {
            resource.close()
            return false
        }
        current = resource
        return true
    }

    fun release() {
        val resource = current
        current = null
        resource?.close()
    }
}
