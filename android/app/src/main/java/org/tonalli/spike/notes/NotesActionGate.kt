package org.tonalli.spike.notes

import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicInteger

/** The same epoch guards both queued work and its later secret-bearing UI result. */
class NotesActionGate {
    private val active = AtomicBoolean(false)
    private val generation = AtomicInteger(0)
    fun resume() { active.set(true) }
    fun pause() { active.set(false); generation.incrementAndGet() }
    fun begin(): Int = generation.incrementAndGet()
    fun permits(stamp: Int): Boolean = active.get() && generation.get() == stamp
}
