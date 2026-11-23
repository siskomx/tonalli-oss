package org.tonalli.spike.reader

import org.tonalli.spike.saf.SafDocument
import org.tonalli.spike.saf.SafSelections

/** Chooser only: Core still validates split completeness, archive content and checksum. */
object ReaderFolderPolicy {
    const val FILES = 81
    const val TREE = 83
    fun restoreRequest(value: Int): Int? = value.takeIf { it == FILES || it == TREE }
    fun acceptsResult(pending: Int?, returned: Int): Boolean =
        restoreRequest(returned) != null && pending == returned

    fun groups(children: List<SafDocument>): List<List<SafDocument>> =
        SafSelections.groups(children).filterNot { group ->
            group.size == 1 && group.single().name.endsWith(".iso", ignoreCase = true)
        }
}
