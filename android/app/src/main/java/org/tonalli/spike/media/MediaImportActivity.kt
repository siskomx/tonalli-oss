package org.tonalli.spike.media

import android.app.Activity
import android.content.Intent
import android.net.Uri
import android.os.Bundle
import android.provider.DocumentsContract
import android.widget.Button
import android.widget.LinearLayout
import android.widget.TextView
import java.io.File
import java.io.IOException
import java.util.concurrent.Executors
import org.tonalli.spike.R
import org.tonalli.spike.saf.SafDocument
import org.tonalli.spike.saf.SafDocuments
import uniffi.tonalli_core.MediaImportHandle
import uniffi.tonalli_core.mediaImportBegin
import uniffi.tonalli_core.mediaImportCancel
import uniffi.tonalli_core.mediaImportFd

/** A one-use native job survives reader/picker Activity recreation without retaining an Admin password. */
class MediaImportActivity:Activity() {
    private val worker=Executors.newSingleThreadExecutor()
    private val nativeLock=Any()
    private var handle:MediaImportHandle?=null
    @Volatile private var active=false
    @Volatile private var destroyed=false
    @Volatile private var stopped=false
    @Volatile private var working=false
    private var pendingRequest:Int?=null
    private var pending:Pair<Int,Intent>?=null
    private lateinit var status:TextView
    private lateinit var files:Button
    private lateinit var folder:Button
    private lateinit var back:Button

    override fun onCreate(state:Bundle?) {
        super.onCreate(state)
        if (intent.getStringExtra("ticket")?.matches(Regex("[a-f0-9]{64}"))!=true) {finish();return}
        pendingRequest=state?.getInt("pending_picker")?.takeIf { it==FILES||it==TREE }
        status=TextView(this).apply {setText(R.string.media_import_help);setPadding(24,24,24,24)}
        files=Button(this).apply {setText(R.string.reader_choose_files);setOnClickListener {pick(FILES)}}
        folder=Button(this).apply {setText(R.string.reader_choose_folder);setOnClickListener {pick(TREE)}}
        back=Button(this).apply {setText(R.string.media_back_files);setOnClickListener {if(working)stop()else finish()}}
        setContentView(LinearLayout(this).apply {orientation=LinearLayout.VERTICAL;addView(status);addView(files);addView(folder);addView(back)})
    }
    private fun pick(request:Int) {
        if(working)return
        pendingRequest=request
        val picker=if(request==TREE)Intent(Intent.ACTION_OPEN_DOCUMENT_TREE)else Intent(Intent.ACTION_OPEN_DOCUMENT).apply {
            addCategory(Intent.CATEGORY_OPENABLE);type="*/*";putExtra(Intent.EXTRA_ALLOW_MULTIPLE,true)
        }
        picker.addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
        try {startActivityForResult(picker,request)}catch(_:Exception){pendingRequest=null;status.setText(R.string.media_import_failed)}
    }
    @Deprecated("Native document picker result")
    override fun onActivityResult(requestCode:Int,resultCode:Int,data:Intent?) {
        super.onActivityResult(requestCode,resultCode,data)
        if(requestCode!=pendingRequest)return
        pendingRequest=null
        if(resultCode!=RESULT_OK||data==null){status.setText(R.string.reader_cancelled);return}
        pending=requestCode to data
        if(active)consume()
    }
    private fun selections(request:Int,result:Intent):List<MediaFolders.Selected> {
        val documents=SafDocuments(contentResolver)
        if(request==FILES) {
            val uris=result.clipData?.let {clip->if(clip.itemCount !in 1..100)throw IOException("Choose up to 100 files");(0 until clip.itemCount).map {clip.getItemAt(it).uri}}
                ?:listOf(result.data?:throw IOException("Choose a file"))
            return uris.map {uri->
                if(uri.toString().length>4096)throw IOException("Document identity is too large")
                val document=documents.document(uri)
                if(document.name.toByteArray(Charsets.UTF_8).size>240)throw IOException("Document name is too large")
                MediaFolders.Selected(MediaFolders.Entry(document.uri,document.name,false),"")
            }
        }
        val tree=result.data?.takeIf {it.scheme=="content"}?:throw IOException("Choose a folder")
        val root=DocumentsContract.buildDocumentUriUsingTree(tree,DocumentsContract.getTreeDocumentId(tree))
        val rootDocument=documents.document(root)
        return MediaFolders.select(MediaFolders.Entry(root.toString(),rootDocument.name,true),{parent->
            val children=DocumentsContract.buildChildDocumentsUriUsingTree(tree,DocumentsContract.getDocumentId(Uri.parse(parent)))
            val entries=mutableListOf<MediaFolders.Entry>()
            contentResolver.query(children,arrayOf(DocumentsContract.Document.COLUMN_DOCUMENT_ID,DocumentsContract.Document.COLUMN_DISPLAY_NAME,DocumentsContract.Document.COLUMN_MIME_TYPE),null,null,null)?.use {cursor->
                while(cursor.moveToNext()) {
                    if(stopped||destroyed||entries.size>=10000)throw IOException("Folder import stopped or too large")
                    val id=cursor.getString(0)?:throw IOException("Missing document identity")
                    val name=cursor.getString(1)?:throw IOException("Missing document name")
                    if(id.length>4096||name.toByteArray(Charsets.UTF_8).size>240)throw IOException("Document metadata is too large")
                    entries.add(MediaFolders.Entry(DocumentsContract.buildDocumentUriUsingTree(tree,id).toString(),name,cursor.getString(2)==DocumentsContract.Document.MIME_TYPE_DIR))
                }
            }?:throw IOException("Folder unavailable")
            entries
        },{stopped||destroyed})
    }
    private fun consume() {
        val (request,result)=pending?:return
        pending=null;working=true;stopped=false;window.addFlags(android.view.WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON);files.isEnabled=false;folder.isEnabled=false;back.setText(R.string.media_stop_import);status.setText(R.string.media_import_reading)
        worker.execute {
            var added=0;var failed=0
            try {
                val owned=mediaImportBegin(File(filesDir,"core").absolutePath,intent.getStringExtra("ticket")?:"")
                synchronized(nativeLock){handle=owned;if(stopped||destroyed)mediaImportCancel(owned)}
                val selected=selections(request,result)
                val documents=SafDocuments(contentResolver)
                for ((index,selection) in selected.withIndex()) {
                    if(stopped||destroyed)break
                    runOnUiThread {if(active&&!destroyed)status.text=getString(R.string.media_import_progress,index+1,selected.size)}
                    try {
                        val descriptor=documents.open(SafDocument(selection.document.id,selection.document.name))
                        val fd=descriptor.use {it.detachFd()}
                        mediaImportFd(owned,selection.document.name,selection.folder,fd);added++
                    }catch(_:Exception){if(!stopped&&!destroyed)failed++}
                }
            }catch(_:Exception){if(!stopped&&!destroyed)failed++}
            finally {synchronized(nativeLock){handle?.destroy();handle=null}}
            working=false
            runOnUiThread {if(active&&!destroyed){window.clearFlags(android.view.WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON);status.text=if(stopped)getString(R.string.media_import_stopped,added)else getString(R.string.media_import_result,added,failed);back.setText(R.string.media_back_files);files.isEnabled=false;folder.isEnabled=false}}
        }
    }
    private fun stop(){stopped=true;window.clearFlags(android.view.WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON);synchronized(nativeLock){handle?.let(::mediaImportCancel)}}
    override fun onResume(){super.onResume();active=true;consume()}
    override fun onSaveInstanceState(outState:Bundle){pendingRequest?.let {outState.putInt("pending_picker",it)};super.onSaveInstanceState(outState)}
    override fun onPause(){active=false;window.clearFlags(android.view.WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON);if(working)stop();super.onPause()}
    override fun onDestroy(){destroyed=true;if(working)stop();worker.shutdown();super.onDestroy()}
    companion object {private const val FILES=901;private const val TREE=902}
}
