package org.tonalli.spike.saf

import java.util.concurrent.atomic.AtomicLong

object SafReadFence {
    fun removedEpoch(generation: AtomicLong, expected: Long): Long? =
        if (generation.compareAndSet(expected, expected + 1)) expected + 1 else null
}
