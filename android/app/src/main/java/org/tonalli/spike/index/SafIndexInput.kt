package org.tonalli.spike.index

import android.content.ContentResolver
import android.net.Uri
import android.os.ParcelFileDescriptor
import android.provider.DocumentsContract
import java.io.IOException
import java.nio.channels.FileChannel
import org.tonalli.spike.saf.SafDocument
import org.tonalli.spike.saf.SafDocuments

/** Two exact folder levels, no recursion, URI-to-path conversion or intermediate payload copy. */
class SafIndexInput(private val resolver: ContentResolver, private val tree: Uri) : TrustedIndexInput {
    private val documents = SafDocuments(resolver)
    private val files = mutableMapOf<String, SafDocument>()

    override fun checkContents(spec: IndexTransferSpec) {
        files.clear()
        val paths = mutableListOf<String>()
        val expected = spec.entries.associate { it.path to it.bytes } + ("snapshot.json" to spec.descriptor.size.toLong())
        fun children(parentId: String, prefix: String): String? {
            var generationId: String? = null
            val uri = DocumentsContract.buildChildDocumentsUriUsingTree(tree, parentId)
            resolver.query(uri, arrayOf(DocumentsContract.Document.COLUMN_DOCUMENT_ID,
                DocumentsContract.Document.COLUMN_DISPLAY_NAME, DocumentsContract.Document.COLUMN_MIME_TYPE,
                DocumentsContract.Document.COLUMN_SIZE, DocumentsContract.Document.COLUMN_FLAGS), null, null, null)?.use { cursor ->
                while (cursor.moveToNext()) {
                    if (paths.size >= IndexTransferSpec.MAX_FILES + 2) throw IOException("Index folder has too many entries")
                    val name = cursor.getString(1) ?: throw IOException("Missing index filename")
                    if (name.length !in 1..160 || name.any { it == '/' || it == '\\' || it.code < 32 || it.code == 127 }) throw IOException("Invalid index filename")
                    val path = prefix + name
                    if (path in paths) throw IOException("Duplicate index filename")
                    paths.add(path)
                    val directory = cursor.getString(2) == DocumentsContract.Document.MIME_TYPE_DIR
                    val id = cursor.getString(0) ?: throw IOException("Missing provider document")
                    if (directory) {
                        if (prefix.isNotEmpty() || name != spec.generation) throw IOException("Unexpected index subfolder")
                        generationId = id
                    } else {
                        val bytes = expected[path] ?: throw IOException("Unlisted index file")
                        if (cursor.getInt(4) and DocumentsContract.Document.FLAG_VIRTUAL_DOCUMENT != 0 ||
                            (!cursor.isNull(3) && cursor.getLong(3) != bytes)) throw IOException("Invalid index file metadata")
                        files[path] = SafDocument(DocumentsContract.buildDocumentUriUsingTree(tree, id).toString(), name)
                    }
                }
            } ?: throw IOException("Provider cannot list the index folder")
            return generationId
        }
        val generation = children(DocumentsContract.getTreeDocumentId(tree), "") ?: throw IOException("Index generation folder is missing")
        children(generation, "${spec.generation}/")
        TrustedIndexInput.requirePaths(spec, paths)
    }

    override fun read(path: String, consume: (FileChannel) -> Unit) {
        val document = files[path] ?: throw IOException("Index file was not selected")
        ParcelFileDescriptor.AutoCloseInputStream(documents.open(document)).use { input -> consume(input.channel) }
    }
}
