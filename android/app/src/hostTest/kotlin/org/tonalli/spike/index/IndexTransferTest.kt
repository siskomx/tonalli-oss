package org.tonalli.spike.index

import java.io.IOException
import java.io.RandomAccessFile
import java.nio.file.Files
import java.nio.file.Path
import java.security.MessageDigest

private fun hash(path: Path): String {
    val digest = MessageDigest.getInstance("SHA-256")
    Files.newInputStream(path).use { input -> val buffer = ByteArray(65536); while (true) {
        val n = input.read(buffer); if (n < 0) break; digest.update(buffer, 0, n)
    } }
    return digest.digest().joinToString("") { "%02x".format(it) }
}
private fun rejects(work: () -> Unit) { try { work(); error("Expected typed rejection") } catch (_: IOException) { } }

fun main() {
    trustedIndexInputTests()
    checkIndexQuerySeries()
    val root = Files.createTempDirectory("trusted-index-test-")
    try {
        val incoming = Files.createDirectory(root.resolve("incoming"))
        val generation = "generation-trusted"
        Files.createDirectory(incoming.resolve(generation))
        val raw = "trusted descriptor fixture".toByteArray()
        Files.write(incoming.resolve("snapshot.json"), raw)
        Files.writeString(incoming.resolve("CURRENT"), "$generation\n")
        val payload = incoming.resolve("$generation/segment.store")
        RandomAccessFile(payload.toFile(), "rw").use { it.setLength(33L * 1024 * 1024); it.seek(it.length() - 1); it.write(7) }
        Files.writeString(incoming.resolve("$generation/meta.json"), "{}")
        Files.writeString(incoming.resolve("$generation/tonalli-meta.json"), "{}")
        val entries = listOf("CURRENT", "$generation/segment.store", "$generation/meta.json", "$generation/tonalli-meta.json").map {
            IndexTransferSpec.Entry(it, Files.size(incoming.resolve(it)), hash(incoming.resolve(it)))
        }
        val query = IndexTransferSpec.Query("solar system", listOf(IndexTransferSpec.Hit("Trefoil knot", "C/Trefoil_knot")))
        val queries = listOf(query, IndexTransferSpec.Query("photosynthesis", emptyList()), IndexTransferSpec.Query("Mexican history", query.hits))
        val spec = IndexTransferSpec.checked("small", raw, generation,
            IndexTransferSpec.Source("a".repeat(32), 14093352, "b".repeat(32), "c".repeat(64)), entries, queries)
        val medicine = IndexTransferSpec.checked("medicine", raw, generation, spec.source, entries, queries)
        check(medicine.dataset == "medicine" && medicine.requiredBytes == spec.requiredBytes &&
            medicine.manifestHash == spec.manifestHash)
        for (unknown in listOf("unknown", "medicine-es", "Medicine", "../medicine", "")) {
            rejects { IndexTransferSpec.checked(unknown, raw, generation, spec.source, entries, queries) }
        }
        rejects { IndexTransferSpec.checked("small", raw, generation, spec.source, entries, listOf(query)) }
        rejects { IndexTransferSpec.checked("small", raw, generation, spec.source, entries.filterNot { it.path.endsWith("/tonalli-meta.json") }, queries) }
        rejects { IndexTransferSpec.checked("small", raw, generation, spec.source,
            entries + entries[1].copy(path = "$generation/SEGMENT.store"), queries) }
        rejects { IndexTransferSpec.checked("small", raw, generation, spec.source,
            entries.map { if (it == entries[1]) it.copy(path = "${it.path}.") else it }, queries) }
        rejects { IndexTransferSpec.checked("small", raw, generation, spec.source, entries,
            queries.map { if (it == query) it.copy(hits = listOf(IndexTransferSpec.Hit("", "C/x"))) else it }) }
        rejects { IndexTransferSpec.checked("small", raw, generation, spec.source, entries,
            queries.map { if (it == query) it.copy(hits = List(6) { query.hits.first() }) else it }) }
        IndexTransferSpec.checked("small", raw, generation, spec.source, entries,
            queries.map { if (it == query) it.copy(hits = listOf(IndexTransferSpec.Hit("á".repeat(4096), "C/x"))) else it })
        check(spec.payloadBytes > 32L * 1024 * 1024)
        check(spec.requiredBytes == spec.payloadBytes * 2 + raw.size + 33554432L)
        val internal = root.resolve("installed")
        val old = Files.createDirectories(internal.resolve("snapshot-old"))
        Files.writeString(old.resolve("marker"), "preserved")
        val remainingStageSpace = spec.payloadBytes + IndexTransferSpec.RESERVE_BYTES
        rejects { TrustedIndexInstaller.install(incoming, internal, spec, { false }, space = { remainingStageSpace - 1 }, validate = {}) }
        check(Files.readString(old.resolve("marker")) == "preserved")
        var validated = false
        val installed = TrustedIndexInstaller.install(incoming, internal, spec, { false }, space = { remainingStageSpace }, validate = { staged ->
            check(!Files.exists(internal.resolve("snapshot-${spec.manifestHash}"))); check(Files.size(staged.resolve("$generation/segment.store")) == entries[1].bytes); validated = true
        })
        check(validated && Files.isDirectory(installed) && Files.readString(old.resolve("marker")) == "preserved")
        var reused = false
        check(TrustedIndexInstaller.install(incoming, internal, spec, { false }, validate = { reused = true }) == installed && reused)
        IndexTransferSpec.requireHits(query, listOf(IndexTransferSpec.Hit("Trefoil knot", "C/Trefoil_knot")))
        rejects { IndexTransferSpec.requireHits(query, listOf(IndexTransferSpec.Hit("Trefoil knot", "C/Wrong_url"))) }
        val other = root.resolve("rejected")
        Files.createDirectory(other)
        Files.writeString(other.resolve("old"), "unchanged")
        var cancel = false
        rejects { TrustedIndexInstaller.install(incoming, other, spec, { cancel }, space = { Long.MAX_VALUE }, validate = { cancel = true }) }
        rejects { TrustedIndexInstaller.install(incoming, other, spec, { false }, space = { Long.MAX_VALUE }, validate = { throw IOException("Ordered URL parity mismatch") }) }
        rejects { TrustedIndexInstaller.install(incoming, other, spec, { false }, space = { Long.MAX_VALUE }, withSelection = { throw IOException("Selection revision changed") }, validate = {}) }
        RandomAccessFile(payload.toFile(), "rw").use { it.seek(0); it.write(9) }
        rejects { TrustedIndexInstaller.install(incoming, other, spec, { false }, space = { Long.MAX_VALUE }, validate = {}) }
        check(Files.readString(other.resolve("old")) == "unchanged")
        check(Files.list(other).use { stream -> stream.noneMatch { it.fileName.toString().startsWith(".stage-") } })
        rejects { IndexTransferSpec.checked("full", raw, generation, spec.source,
            listOf(IndexTransferSpec.Entry("CURRENT", 1, "a".repeat(64)), IndexTransferSpec.Entry("$generation/meta.json", Long.MAX_VALUE, "b".repeat(64)),
                IndexTransferSpec.Entry("$generation/tonalli-meta.json", 1, "c".repeat(64))), queries) }
        rejects { IndexTransferSpec.checked("small", raw, "../escape", spec.source, entries, queries) }
        println("Trusted index >32MiB profile, checked space/overflow, staged validation, cancellation/corruption/parity/reselection preservation passed")
    } finally { Files.walk(root).use { it.sorted(Comparator.reverseOrder()).forEach(Files::deleteIfExists) } }
}
