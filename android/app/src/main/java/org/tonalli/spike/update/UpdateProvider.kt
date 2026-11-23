package org.tonalli.spike.update

import android.content.ContentProvider
import android.content.ContentValues
import android.content.Context
import android.database.Cursor
import android.database.MatrixCursor
import android.net.Uri
import android.os.ParcelFileDescriptor
import android.provider.OpenableColumns
import java.io.File
import java.io.FileNotFoundException

/**
 * Hands the system installer exactly one thing: the downloaded update, and only if Core verifies it
 * again (signature, size and SHA-256) at the moment it is read. Not exported; access is granted per
 * Intent. Nothing else in app storage is reachable through it.
 */
class UpdateProvider : ContentProvider() {
    override fun onCreate() = true

    private fun verified(): uniffi.tonalli_core.UpdateReady? {
        val context = context ?: return null
        return try { uniffi.tonalli_core.updateReady(File(context.filesDir, "core").absolutePath) } catch (_: Exception) { null }
    }

    private fun ours(uri: Uri) = uri.path == PATH

    override fun getType(uri: Uri): String? = if (ours(uri)) APK_MIME else null

    override fun query(uri: Uri, projection: Array<out String>?, selection: String?, selectionArgs: Array<out String>?, sortOrder: String?): Cursor? {
        if (!ours(uri)) return null
        val ready = verified() ?: return null
        val columns = (projection ?: arrayOf(OpenableColumns.DISPLAY_NAME, OpenableColumns.SIZE)).filter { it == OpenableColumns.DISPLAY_NAME || it == OpenableColumns.SIZE }
        return MatrixCursor(columns.toTypedArray()).apply {
            addRow(columns.map { if (it == OpenableColumns.SIZE) ready.bytes else NAME }.toTypedArray())
        }
    }

    override fun openFile(uri: Uri, mode: String): ParcelFileDescriptor {
        if (!ours(uri) || mode != "r") throw FileNotFoundException()
        val ready = verified() ?: throw FileNotFoundException()
        return ParcelFileDescriptor.open(File(ready.path), ParcelFileDescriptor.MODE_READ_ONLY)
    }

    override fun insert(uri: Uri, values: ContentValues?): Uri? = throw UnsupportedOperationException()
    override fun update(uri: Uri, values: ContentValues?, selection: String?, selectionArgs: Array<out String>?): Int = throw UnsupportedOperationException()
    override fun delete(uri: Uri, selection: String?, selectionArgs: Array<out String>?): Int = throw UnsupportedOperationException()

    companion object {
        const val APK_MIME = "application/vnd.android.package-archive"
        private const val PATH = "/package.apk"
        private const val NAME = "Tonalli-update.apk"
        fun uri(context: Context): Uri = Uri.parse("content://${context.packageName}.update$PATH")
    }
}
