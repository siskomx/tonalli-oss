package org.tonalli.spike.ai

/** Callback/UI state only. Returned cancellation IDs are acted on after leaving the monitor. */
class AiRunGate {
    data class Started(val accepted: Boolean, val cancelId: ULong?)
    private var epoch = 0L
    private var active = false
    private var id: ULong? = null
    private var cancelling = false
    private var terminal = false

    @Synchronized fun resume() { active = true }

    @Synchronized fun begin(): Long {
        check(active)
        epoch = Math.incrementExact(epoch)
        id = null
        cancelling = false
        terminal = false
        return epoch
    }

    @Synchronized fun permits(stamp: Long): Boolean = active && stamp == epoch
    @Synchronized fun cancelled(stamp: Long): Boolean = !permits(stamp) || cancelling

    @Synchronized fun started(stamp: Long, value: ULong): Started {
        if (!permits(stamp) || terminal || (id != null && id != value)) return Started(false, value)
        id = value
        return Started(true, if (cancelling) value else null)
    }

    @Synchronized fun returned(stamp: Long, value: ULong): ULong? {
        if (!permits(stamp) || (id != null && id != value)) return value
        id = value
        return if (cancelling && !terminal) value else null
    }

    @Synchronized fun text(stamp: Long, value: ULong): Boolean =
        permits(stamp) && !terminal && !cancelling && id == value

    @Synchronized fun finished(stamp: Long, value: ULong): Boolean {
        if (!permits(stamp) || terminal || id != value) return false
        terminal = true
        return true
    }

    @Synchronized fun cancel(): ULong? {
        cancelling = true
        return if (active && !terminal) id else null
    }

    @Synchronized internal fun cancel(stamp: Long): ULong? = if (permits(stamp)) cancel() else null

    @Synchronized fun depart(): ULong? {
        active = false
        cancelling = true
        epoch = Math.incrementExact(epoch)
        return if (!terminal) id else null
    }
}
