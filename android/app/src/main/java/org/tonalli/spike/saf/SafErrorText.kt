package org.tonalli.spike.saf

/** Expose only the bounded split suffix, never arbitrary native/provider detail. */
object SafErrorText {
    fun missingSplitPart(reason: String): String? {
        val suffix = Regex("duplicate part or missing part \\.zim([a-z]{2})").matchEntire(reason)
            ?.groupValues?.get(1) ?: return null
        return "Duplicate part or missing part .zim$suffix; select the complete group and reopen."
    }
}
