package org.tonalli.spike.index

/** Intent numbers remain exact across the full Core u64 range. */
internal fun preparationIdentity(pack: String?, epoch: String?): Pair<String, ULong>? {
    if (pack == null || !Regex("[A-Za-z0-9_-]{1,128}").matches(pack) ||
        epoch == null || !Regex("[1-9][0-9]{0,19}").matches(epoch)) return null
    val number = epoch.toULongOrNull() ?: return null
    return Pair(pack, number)
}
