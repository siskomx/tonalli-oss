package org.tonalli.spike.saf

import android.content.ContentResolver
import android.content.Intent
import android.net.Uri
import android.os.ParcelFileDescriptor
import android.provider.DocumentsContract
import android.provider.OpenableColumns
import android.system.Os
import android.system.OsConstants
import java.io.IOException

/** Content URIs remain opaque: no filesystem path conversion or recursive traversal. */
class SafDocuments(private val resolver: ContentResolver) {
    fun persist(uri: Uri, returnedFlags: Int) {
        if (uri.scheme != "content" || returnedFlags and Intent.FLAG_GRANT_READ_URI_PERMISSION == 0 ||
            returnedFlags and Intent.FLAG_GRANT_PERSISTABLE_URI_PERMISSION == 0) {
            throw IOException("Provider did not return a persistable read grant; choose another provider")
        }
        resolver.takePersistableUriPermission(uri, Intent.FLAG_GRANT_READ_URI_PERMISSION)
        requireGrant(uri)
    }

    fun document(uri: Uri): SafDocument {
        if (uri.scheme != "content") throw IOException("Only SAF content documents are supported")
        val name = resolver.query(uri, arrayOf(OpenableColumns.DISPLAY_NAME), null, null, null)?.use { cursor ->
            if (!cursor.moveToFirst()) throw IOException("Document metadata row is unavailable")
            if (cursor.isNull(0)) "" else cursor.getString(0)
        } ?: throw IOException("Provider metadata is unavailable")
        return SafDocument(uri.toString(), name)
    }

    fun children(tree: Uri): List<SafDocument> {
        requireGrant(tree)
        val children = DocumentsContract.buildChildDocumentsUriUsingTree(tree, DocumentsContract.getTreeDocumentId(tree))
        val result = mutableListOf<SafDocument>()
        resolver.query(children, arrayOf(DocumentsContract.Document.COLUMN_DOCUMENT_ID,
            DocumentsContract.Document.COLUMN_DISPLAY_NAME, DocumentsContract.Document.COLUMN_MIME_TYPE), null, null, null)?.use { cursor ->
            var count = 0
            while (cursor.moveToNext()) {
                if (++count > 256) throw IOException("Folder has over 256 immediate children; select files directly or choose a smaller folder")
                if (cursor.getString(2) != DocumentsContract.Document.MIME_TYPE_DIR) {
                    val uri = DocumentsContract.buildDocumentUriUsingTree(tree, cursor.getString(0))
                    result.add(SafDocument(uri.toString(), cursor.getString(1) ?: throw IOException("Missing child display name")))
                }
            }
        } ?: throw IOException("Provider cannot enumerate this folder")
        return result
    }

    fun resolve(selection: SafSelection): List<SafDocument> {
        SafSelections.checked(selection)
        if (selection.treeUri != null) {
            val current = children(Uri.parse(selection.treeUri)).associateBy { it.uri }
            return selection.documents.map { expected ->
                val actual = current[expected.uri] ?: throw IOException("Selected folder document is missing; reselect or reinsert media")
                if (actual.name != expected.name) throw IOException("Selected document name changed; select the group again")
                actual
            }
        }
        return selection.documents.map { expected ->
            val uri = Uri.parse(expected.uri)
            requireGrant(uri)
            document(uri).also { if (it.name != expected.name) throw IOException("Selected document name changed; select again") }
        }
    }

    fun open(document: SafDocument): ParcelFileDescriptor {
        val descriptor = resolver.openFileDescriptor(Uri.parse(document.uri), "r")
            ?: throw IOException("Provider returned no readable descriptor")
        try {
            // Pipes/cloud streams are not a positioned-read source. The core owns accepted FDs.
            Os.lseek(descriptor.fileDescriptor, 0, OsConstants.SEEK_CUR)
            return descriptor
        } catch (error: Throwable) {
            descriptor.close()
            throw IOException("Provider descriptor does not support positioned reads", error)
        }
    }

    private fun requireGrant(uri: Uri) {
        if (resolver.persistedUriPermissions.none { it.uri == uri && it.isReadPermission }) {
            throw SecurityException("Persisted read permission is missing or revoked; select the document again")
        }
    }
}
