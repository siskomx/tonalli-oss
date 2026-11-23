package org.tonalli.spike.web

import java.io.IOException
import org.tonalli.spike.media.MediaFolders

fun mediaFolderTests() {
    val root=MediaFolders.Entry("root","Movies",true)
    val folders=mapOf("root" to listOf(MediaFolders.Entry("season","Season 1",true),MediaFolders.Entry("cover","cover.jpg",false)),"season" to listOf(MediaFolders.Entry("episode","one.mp4",false)))
    val selected=MediaFolders.select(root,{folders[it]?:emptyList()},{false})
    check(selected.map { it.folder+"/"+it.document.name }.sorted()==listOf("Movies/Season 1/one.mp4","Movies/cover.jpg"))
    fun rejects(block:()->Unit){try {block()}catch(_:IOException){return};error("Expected bounded selection failure")}
    rejects { MediaFolders.select(root,{listOf(root)},{false}) }
    rejects { MediaFolders.select(root,{emptyList()},{true}) }
    rejects { MediaFolders.select(root,{id->listOf(MediaFolders.Entry(id+"x","nested",true))},{false}) }
    rejects { MediaFolders.select(root,{(0..10000).map { MediaFolders.Entry("id$it","file$it.txt",false) }},{false}) }
    println("PASS: nested managed-folder names, cycle/depth/count bounds and cancellation")
}
