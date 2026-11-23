package org.tonalli.spike.media

import java.io.IOException

/** Opaque provider identities and virtual folder labels; never filesystem paths. */
object MediaFolders {
    data class Entry(val id:String,val name:String,val directory:Boolean)
    data class Selected(val document:Entry,val folder:String)
    fun select(root:Entry,children:(String)->List<Entry>,cancelled:()->Boolean):List<Selected> {
        if (!root.directory) throw IOException("Choose a folder")
        val result=mutableListOf<Selected>()
        val seen=mutableSetOf<String>()
        val queue=ArrayDeque<Triple<Entry,String,Int>>()
        queue.add(Triple(root,root.name,1))
        var count=0
        while (queue.isNotEmpty()) {
            if (cancelled()) throw IOException("Import stopped")
            val (entry,folder,depth)=queue.removeFirst()
            if (++count>20000||depth>16||folder.toByteArray(Charsets.UTF_8).size>1024||entry.id.isEmpty()||entry.id.length>4096||
                entry.name.isEmpty()||entry.name.toByteArray(Charsets.UTF_8).size>240||entry.name in setOf(".","..")||entry.name.any { it<' '||it=='/'||it=='\\' }||!seen.add(entry.id)) throw IOException("Folder is too large, deep or inconsistent")
            if (entry.directory) {
                val entries=children(entry.id)
                if (entries.size>10000) throw IOException("Folder has too many entries")
                entries.forEach { child -> queue.add(Triple(child,if (child.directory) "$folder/${child.name}" else folder,depth+if(child.directory)1 else 0)) }
            } else {
                if (result.size>=10000) throw IOException("Choose a smaller folder")
                result.add(Selected(entry,folder))
            }
        }
        return result
    }
}
