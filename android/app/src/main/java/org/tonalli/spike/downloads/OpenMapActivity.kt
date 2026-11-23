package org.tonalli.spike.downloads

import android.app.Activity
import android.content.Intent
import android.os.Bundle
import android.provider.OpenableColumns
import android.widget.Button
import android.widget.LinearLayout
import android.widget.TextView
import java.util.concurrent.Executors
import org.tonalli.spike.R
import uniffi.tonalli_core.*

/** Copies a selected map into private verified storage; never modifies the provider file. */
class OpenMapActivity : Activity() {
    private val worker = Executors.newSingleThreadExecutor()
    private lateinit var status: TextView
    override fun onCreate(state: Bundle?) {
        super.onCreate(state)
        status = TextView(this).apply { setText(R.string.map_import_help); setPadding(24,24,24,24) }
        setContentView(LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL; addView(status)
            addView(Button(this@OpenMapActivity).apply { setText(R.string.map_import_choose); setOnClickListener {
                startActivityForResult(Intent(Intent.ACTION_OPEN_DOCUMENT).apply { addCategory(Intent.CATEGORY_OPENABLE); type="*/*"; addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION) }, 7105)
            } })
            addView(Button(this@OpenMapActivity).apply { setText(R.string.reader_back_library); setOnClickListener {finish()} })
        })
    }
    @Deprecated("Native document picker")
    override fun onActivityResult(requestCode: Int, resultCode: Int, data: Intent?) {
        super.onActivityResult(requestCode,resultCode,data)
        if (requestCode != 7105 || resultCode != RESULT_OK) return
        val uri = data?.data ?: return
        status.setText(R.string.map_import_running)
        worker.execute {
            var owner: MapsHandle? = null
            var source: PackHandle? = null
            try {
                val name = contentResolver.query(uri,arrayOf(OpenableColumns.DISPLAY_NAME),null,null,null)?.use { cursor -> if(cursor.moveToFirst()) cursor.getString(0) else null } ?: "Map file"
                val descriptor = contentResolver.openFileDescriptor(uri,"r") ?: throw java.io.IOException("Map unavailable")
                descriptor.use { source = packOpenFds(listOf(PackPart(name,it.detachFd()))) }
                owner = NativeMaps.open(applicationContext)
                mapsImport(owner, source!!, name)
                runOnUiThread { if (!isFinishing && !isDestroyed) {setResult(RESULT_OK);finish()} }
            } catch (_: Exception) { runOnUiThread { if (!isFinishing && !isDestroyed) status.setText(R.string.map_import_failed) } }
            finally { source?.let { try {packClose(it)} catch (_: Exception) {} ;it.destroy() };owner?.destroy() }
        }
    }
    override fun onDestroy() {worker.shutdown();super.onDestroy()}
}
