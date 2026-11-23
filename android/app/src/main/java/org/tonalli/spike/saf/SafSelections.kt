package org.tonalli.spike.saf

import java.io.IOException

enum class SafKind { FILES, TREE, ISO }
data class SafDocument(val uri: String, val name: String)
data class SafSelection(val kind: SafKind, val documents: List<SafDocument>, val treeUri: String? = null, val innerPath: String? = null)

object SafSelections {
    private fun contentUri(uri: String) {
        if (!uri.startsWith("content://") || uri.length <= 10 || uri.toByteArray(Charsets.UTF_8).size > 8192 || uri.any(Char::isISOControl)) {
            throw IOException("Invalid content document URI")
        }
    }
    private fun document(document: SafDocument) {
        contentUri(document.uri)
        if (document.name.toByteArray(Charsets.UTF_8).size > 4096 || document.name.any(Char::isISOControl)) throw IOException("Invalid display name")
    }
    fun checked(selection: SafSelection): SafSelection {
        if (selection.documents.isEmpty() || selection.documents.size > 128) throw IOException("Select between 1 and 128 parts")
        selection.documents.forEach(::document)
        if (selection.documents.size > 1 && selection.documents.any { it.name.isEmpty() }) throw IOException("Split parts require display names")
        selection.treeUri?.let(::contentUri)
        if (selection.kind == SafKind.TREE && selection.treeUri == null) throw IOException("Folder selection requires its tree URI")
        if (selection.kind == SafKind.ISO) {
            if (selection.documents.size != 1) throw IOException("Select one optical image")
            val path = selection.innerPath ?: throw IOException("Explicit ISO inner path is required")
            val relative = path.removePrefix("/")
            val parts = relative.split('/')
            if (relative.isEmpty() || path.toByteArray(Charsets.UTF_8).size > 4096 || path.any(Char::isISOControl) || '\\' in path ||
                parts.size > 64 || parts.any { it.isEmpty() || it == "." || it == ".." }) throw IOException("Invalid optical image path")
        } else if (selection.innerPath != null) throw IOException("Inner path is only valid for an optical image")
        return selection
    }

    /** Chooser grouping only; Rust validates completeness, part sizes, suffixes and order. */
    fun groups(children: List<SafDocument>): List<List<SafDocument>> {
        if (children.size > 256) throw IOException("Folder exceeds 256 immediate documents")
        val groups = linkedMapOf<String, MutableList<SafDocument>>()
        for ((index, child) in children.withIndex()) {
            document(child)
            val name = child.name
            val key = when {
                name.endsWith(".zim") || name.endsWith(".iso", ignoreCase = true) -> "single:$index"
                Regex(".*\\.zim[a-z]{2}").matches(name) -> "split:${name.dropLast(2)}"
                else -> continue
            }
            val group = groups.getOrPut(key) { mutableListOf() }
            group.add(child)
            if (group.size > 128 || groups.size > 128) throw IOException("Folder exceeds chooser group limits")
        }
        return groups.values.map { it.toList() }
    }
}
