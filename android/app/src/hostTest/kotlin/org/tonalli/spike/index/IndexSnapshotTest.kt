package org.tonalli.spike.index

import java.io.IOException
import java.nio.file.Files
import java.nio.file.Path
import java.security.MessageDigest

private fun digest(bytes: ByteArray) = MessageDigest.getInstance("SHA-256")
    .digest(bytes).joinToString("") { "%02x".format(it) }

private fun rejects(block: () -> Unit) {
    try { block() } catch (_: IOException) { return }
    error("Expected IOException")
}

fun main() {
    check(IndexSnapshot.incomingName("a".repeat(64)) == "probe-index-aaaaaaaaaaaa")
    check(IndexSnapshot.incomingName("a".repeat(64)) != IndexSnapshot.incomingName("b".repeat(64)))
    rejects { IndexSnapshot.incomingName("../escape") }
    val root = Files.createTempDirectory("tonalli-index-test-")
    try {
        val incoming = Files.createDirectory(root.resolve("incoming"))
        val generation = "generation-test"
        Files.createDirectory(incoming.resolve(generation))
        val selector = "$generation\n".toByteArray()
        val payload = "checked index payload".toByteArray()
        Files.write(incoming.resolve("CURRENT"), selector)
        Files.write(incoming.resolve("$generation/data.idx"), payload)
        val entries = listOf(
            IndexSnapshot.Entry("CURRENT", selector.size.toLong(), digest(selector)),
            IndexSnapshot.Entry("$generation/data.idx", payload.size.toLong(), digest(payload)),
        )
        val internal = Files.createDirectory(root.resolve("internal"))
        val old = Files.createDirectory(internal.resolve("snapshot-old"))
        Files.writeString(old.resolve("sentinel"), "keep")
        var validationCalls = 0
        val installed = IndexSnapshot.install(incoming, internal, "a".repeat(64), generation, entries) {
            check(Files.readAllBytes(it.resolve("$generation/data.idx")).contentEquals(payload))
            check(!Files.exists(internal.resolve("snapshot-" + "a".repeat(64))))
            validationCalls++
        }
        check(validationCalls == 1 && Files.isDirectory(installed))
        check(Files.readString(old.resolve("sentinel")) == "keep")
        check(IndexSnapshot.install(incoming, internal, "a".repeat(64), generation, entries) {} == installed)

        Files.write(incoming.resolve("$generation/data.idx"), payload.copyOf().apply { this[0] = 'X'.code.toByte() })
        rejects { IndexSnapshot.install(incoming, internal, "b".repeat(64), generation, entries) {} }
        check(!Files.exists(internal.resolve("snapshot-" + "b".repeat(64))))
        check(Files.readAllBytes(installed.resolve("$generation/data.idx")).contentEquals(payload))
        Files.writeString(incoming.resolve("$generation/data.idx"), "short")
        rejects { IndexSnapshot.install(incoming, internal, "1".repeat(64), generation, entries) {} }
        Files.write(incoming.resolve("$generation/data.idx"), payload + byteArrayOf(1))
        rejects { IndexSnapshot.install(incoming, internal, "2".repeat(64), generation, entries) {} }
        Files.write(incoming.resolve("$generation/data.idx"), payload)
        rejects { IndexSnapshot.install(incoming, internal, "c".repeat(64), generation, entries) { throw IOException("native validation failed") } }
        check(!Files.exists(internal.resolve("snapshot-" + "c".repeat(64))))
        rejects { IndexSnapshot.install(incoming, internal, "d".repeat(64), generation, entries + IndexSnapshot.Entry("../escape", 0, "0".repeat(64))) {} }
        rejects { IndexSnapshot.install(incoming, internal, "e".repeat(64), generation, entries + IndexSnapshot.Entry("$generation/.tantivy-meta.lock", 0, digest(byteArrayOf()))) {} }
        rejects { IndexSnapshot.install(incoming, internal, "f".repeat(64), generation, entries.map { it.copy(bytes = Long.MAX_VALUE) }) {} }
        check(Files.list(internal).use { paths -> paths.noneMatch { it.fileName.toString().startsWith(".stage-") } })
        println("PASS: verified copy, validate-before-publish, reuse, same-length checksum corruption/truncation/growth preservation, native failure preservation, traversal/lock/size rejection, staging cleanup")
    } finally {
        check(root.toRealPath().parent == Path.of(System.getProperty("java.io.tmpdir")).toRealPath())
        check(root.fileName.toString().startsWith("tonalli-index-test-"))
        Files.walk(root).use { paths -> paths.sorted(Comparator.reverseOrder()).forEach(Files::delete) }
    }
}
