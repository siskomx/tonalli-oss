package org.tonalli.spike.reader

import java.io.IOException
import org.tonalli.spike.saf.SafSelection
import org.tonalli.spike.saf.SafSelections

data class ReaderIdentity(val uuid: String, val storedMd5: String)
data class ReaderEntry(val id: String, val selection: SafSelection, val identity: ReaderIdentity)

object ReaderCatalogPolicy {
    fun checked(entries: List<ReaderEntry>): List<ReaderEntry> {
        if (entries.size > 64 || entries.map { it.id }.distinct().size != entries.size) throw IOException("Invalid reader catalog")
        entries.forEach {
            if (!Regex("reader-[a-f0-9]{8}(?:-[a-f0-9]{4}){3}-[a-f0-9]{12}").matches(it.id)) throw IOException("Invalid reader identity")
            SafSelections.checked(it.selection)
            identity(it.identity)
        }
        return entries.toList()
    }

    private fun identity(value: ReaderIdentity) {
        if (!Regex("[a-f0-9]{32}").matches(value.uuid) || !Regex("[a-f0-9]{32}").matches(value.storedMd5)) throw IOException("Invalid verified archive identity")
    }

    fun requireIdentity(entry: ReaderEntry, actual: ReaderIdentity) {
        identity(actual)
        if (entry.identity != actual) throw IOException("Archive identity changed; use Open content to explicitly verify it again")
    }

    /** Only the explicit import flow may call this after Core checksum verification. */
    fun add(entries: List<ReaderEntry>, selection: SafSelection, verified: ReaderIdentity, id: String): List<ReaderEntry> {
        checked(entries)
        SafSelections.checked(selection)
        identity(verified)
        val prior = entries.firstOrNull { it.selection == selection }
        if (prior != null) {
            if (prior.identity == verified) return entries
            return checked(entries.map { if (it.id == prior.id) it.copy(identity = verified) else it })
        }
        return checked(entries + ReaderEntry(id, selection, verified))
    }
}
