package org.tonalli.spike.reader

import java.nio.file.Files
import org.tonalli.spike.index.IndexTransferSpec

/** Test preparation only: the matching bridge must not select a diagnostic index by proximity. */
fun readerIndexTests() {
    val root = Files.createTempDirectory("reader-index-policy-")
    try {
        val generation = "generation-authored"
        val spec = IndexTransferSpec.checked("medicine", "packaged trusted fixture".toByteArray(), generation,
            IndexTransferSpec.Source("a".repeat(32), 300, "b".repeat(32), "c".repeat(64)),
            listOf(IndexTransferSpec.Entry("CURRENT", 20, "d".repeat(64)),
                IndexTransferSpec.Entry("$generation/meta.json", 2, "e".repeat(64)),
                IndexTransferSpec.Entry("$generation/tonalli-meta.json", 2, "f".repeat(64))),
            IndexTransferSpec.QUERY_NAMES.map { IndexTransferSpec.Query(it, emptyList()) })
        val identity = ReaderIdentity(spec.source.uuid, spec.source.md5)
        Files.createDirectories(root.resolve("index-probe/snapshot-${spec.manifestHash}"))
        check(ReaderIndexPolicy.candidate(root, spec, identity, listOf(100, 200)) == null) {
            "The small diagnostic index must never stand in for a reader index"
        }
        val expected = Files.createDirectories(root.resolve("index-handoff/medicine/snapshot-${spec.manifestHash}"))
        check(ReaderIndexPolicy.candidate(root, spec, identity, listOf(100, 200)) == expected)
        check(ReaderIndexPolicy.candidate(root, spec, identity.copy(uuid = "d".repeat(32)), listOf(300)) == null)
        check(ReaderIndexPolicy.candidate(root, spec, identity.copy(storedMd5 = "d".repeat(32)), listOf(300)) == null)
        for (sizes in listOf(listOf<Long?>(299), listOf<Long?>(null), listOf<Long?>(-1),
            listOf<Long?>(Long.MAX_VALUE, Long.MAX_VALUE), emptyList<Long?>())) {
            check(ReaderIndexPolicy.candidate(root, spec, identity, sizes) == null)
        }
        // Exact manifest-derived generation only; no scanning or latest-directory guess.
        Files.delete(expected)
        Files.createDirectories(expected.parent.resolve("snapshot-${"0".repeat(64)}"))
        check(ReaderIndexPolicy.candidate(root, spec, identity, listOf(300)) == null)
        println("Reader index: exact trusted snapshot, archive identity and source lengths; no probe fallback")
    } finally {
        Files.walk(root).use { paths -> paths.sorted(Comparator.reverseOrder()).forEach(Files::deleteIfExists) }
    }
}
