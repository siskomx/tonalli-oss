package org.tonalli.spike.saf

import java.io.IOException
import java.nio.file.Files
import org.tonalli.spike.index.IndexSnapshot
import org.tonalli.spike.probe.ProbeInputDirectories

fun checkProbeInputs() {
    val hash = "a".repeat(64)
    val generation = "generation-safe"
    val entries = listOf(IndexSnapshot.Entry("CURRENT", 16, "b".repeat(64)),
        IndexSnapshot.Entry("$generation/meta.json", 3, "c".repeat(64)))
    val paths = ProbeInputDirectories.paths(hash, generation, entries, listOf("small", "full"))
    check(paths == listOf("probe-index-aaaaaaaaaaaa", "probe-index-aaaaaaaaaaaa/$generation", "benchmark", "benchmark/full", "benchmark/small"))
    val root = Files.createTempDirectory("probe-input-test-")
    try {
        ProbeInputDirectories.prepare(root, paths) { true }
        check(paths.all { Files.isDirectory(root.resolve(it)) })
        val marker = root.resolve("benchmark/small/operator-input")
        Files.writeString(marker, "preserved")
        try { ProbeInputDirectories.prepare(root, paths) { it != root.resolve("benchmark/small") }; error("Expected wrong owner rejection") }
        catch (_: IOException) { }
        check(Files.readString(marker) == "preserved")
        try { ProbeInputDirectories.paths(hash, "../outside", entries, listOf("small")); error("Expected traversal rejection") }
        catch (_: IOException) { }
        try { ProbeInputDirectories.paths(hash, generation, entries, listOf("../outside")); error("Expected dataset rejection") }
        catch (_: IOException) { }
    } finally { Files.walk(root).use { it.sorted(Comparator.reverseOrder()).forEach(Files::deleteIfExists) } }
    println("App-owned trusted transfer directory/ownership preservation checks passed")
}
