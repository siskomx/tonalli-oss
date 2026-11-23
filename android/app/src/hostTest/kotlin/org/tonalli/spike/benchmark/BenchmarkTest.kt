package org.tonalli.spike.benchmark

import java.io.IOException
import java.nio.file.Files
import java.nio.file.Path
import java.security.MessageDigest
import java.util.concurrent.CountDownLatch
import java.util.concurrent.atomic.AtomicInteger

private fun sha(bytes: ByteArray) = MessageDigest.getInstance("SHA-256").digest(bytes).joinToString("") { "%02x".format(it) }
private fun rejects(block: () -> Unit) { try { block() } catch (_: IOException) { return }; error("Expected IOException") }

fun main() {
    val values = (1..30).map(Int::toDouble)
    check(BenchmarkSeries.nearestRank(values, .5) == 15.0)
    check(BenchmarkSeries.nearestRank(values, .95) == 29.0)
    rejects { BenchmarkSeries.nearestRank(values, Double.NaN) }
    val calls = mutableListOf<String>()
    var now = 0L
    val fresh = BenchmarkSeries.Samples()
    BenchmarkSeries.run("fresh-article", fresh, { false }, { now.also { now += 1_000_000 } },
        { calls.add("suggest"); 1 }, { calls.add("article"); 1 })
    check(calls == listOf("article") && fresh.suggest.isEmpty() && fresh.article == listOf(1.0))
    check(fresh.primedSuggest == 0 && fresh.primedArticle == 0)
    calls.clear()
    val warm = BenchmarkSeries.Samples()
    BenchmarkSeries.run("warm", warm, { false }, { now.also { now += 1_000_000 } },
        { calls.add("suggest"); 1 }, { calls.add("article"); 1 })
    check(warm.suggest.size == 30 && warm.article.size == 30 && calls.count { it == "suggest" } == 31 && calls.count { it == "article" } == 31)
    rejects { BenchmarkSeries.run("fresh-suggest", BenchmarkSeries.Samples(), { true }, { 0L }, { error("Cancelled work ran") }, { 1 }) }
    rejects { BenchmarkRequest.checked("small", "a".repeat(64), "../outside", "warm", 30) }
    rejects { BenchmarkRequest.checked("small", "a".repeat(64), "run1", "warm", 1) }
    check(BenchmarkRequest.checked("small", "a".repeat(64), "run1", "fresh-suggest", 1).mode == "fresh-suggest")
    check(WebBenchmarkRequest.checked("full", "a".repeat(64), "m1-perf-run_1").profile == "m1-perf-run_1")
    rejects { WebBenchmarkRequest.checked("full", "a".repeat(64), "../outside") }
    val launch = DeferredWebLaunch()
    check(!launch.takeLaunch())
    launch.request()
    check(!launch.takeLaunch())
    launch.ready()
    check(launch.takeLaunch())
    launch.request()
    check(!launch.takeLaunch())

    val root = Files.createTempDirectory("tonalli-benchmark-test-")
    try {
        val results = Files.createDirectory(root.resolve("results"))
        val interrupted = BenchmarkFiles.claimResult(results, "interrupted")
        check(!Files.exists(interrupted))
        rejects { BenchmarkFiles.claimResult(results, "interrupted") }
        check(Files.isRegularFile(results.resolve("interrupted.claim")))
        val existing = results.resolve("legacy.json")
        Files.writeString(existing, "retain historical result")
        rejects { BenchmarkFiles.claimResult(results, "legacy") }
        check(Files.readString(existing) == "retain historical result")
        rejects { BenchmarkFiles.claimResult(results, "../escape") }
        val gate = CountDownLatch(1)
        val winners = AtomicInteger()
        val refused = AtomicInteger()
        val writers = List(2) {
            Thread {
                gate.await()
                try { BenchmarkFiles.claimResult(results, "competing"); winners.incrementAndGet() }
                catch (_: IOException) { refused.incrementAndGet() }
            }.apply { start() }
        }
        gate.countDown()
        writers.forEach(Thread::join)
        check(winners.get() == 1 && refused.get() == 1)
        Files.writeString(results.resolve("competing.json"), "winner evidence")
        rejects { BenchmarkFiles.claimResult(results, "competing") }
        check(Files.readString(results.resolve("competing.json")) == "winner evidence")
        val incoming = Files.createDirectory(root.resolve("incoming"))
        val internal = Files.createDirectory(root.resolve("internal"))
        val bytes = ByteArray(128 * 1024 + 7) { (it % 251).toByte() }
        Files.write(incoming.resolve("pack.zim"), bytes)
        Files.write(incoming.resolve("titles.bin"), byteArrayOf(1, 2, 3))
        val entries = listOf(BenchmarkFiles.Entry("pack.zim", bytes.size.toLong(), sha(bytes)),
            BenchmarkFiles.Entry("titles.bin", 3, sha(byteArrayOf(1, 2, 3))))
        val installed = BenchmarkFiles.prepare(incoming, internal, "a".repeat(64), entries, { false }, { Long.MAX_VALUE }) {
            check(!Files.exists(internal.resolve("a".repeat(64))))
        }
        check(BenchmarkFiles.prepared(internal, "a".repeat(64), entries) == installed)
        Files.write(incoming.resolve("pack.zim"), bytes.copyOf().apply { this[0] = 99 })
        rejects { BenchmarkFiles.prepare(incoming, internal, "b".repeat(64), entries, { false }, { Long.MAX_VALUE }) {} }
        check(Files.readAllBytes(installed.resolve("pack.zim")).contentEquals(bytes))
        Files.write(incoming.resolve("pack.zim"), bytes)
        rejects { BenchmarkFiles.prepare(incoming, internal, "c".repeat(64), entries, { true }, { Long.MAX_VALUE }) {} }
        rejects { BenchmarkFiles.prepare(incoming, internal, "d".repeat(64), entries, { false }, { 0L }) {} }
        rejects { BenchmarkFiles.prepare(incoming, internal, "e".repeat(64), entries, { false }, { Long.MAX_VALUE }) { throw IOException("native validation") } }
        var checks = 0
        rejects { BenchmarkFiles.prepare(incoming, internal, "f".repeat(64), entries, { ++checks > 3 }, { Long.MAX_VALUE }) {} }
        check(Files.list(internal).use { it.count() } == 1L)
        val tableInput = Files.createDirectory(root.resolve("table-input"))
        val tables = root.resolve("tables")
        val tableBytes = byteArrayOf(1, 2, 3)
        val table = entries.single { it.name == "titles.bin" }
        Files.write(tableInput.resolve("titles.bin"), tableBytes)
        var validated = 0
        val tableInstalled = BenchmarkFiles.prepareTable(tableInput, tables, "1".repeat(64), table,
            { false }, { 32L * 1024 * 1024 + 3 }) {
            check(!Files.exists(tables.resolve("1".repeat(64))))
            check(Files.readAllBytes(it.resolve("titles.bin")).contentEquals(tableBytes))
            check(!Files.exists(it.resolve("pack.zim")))
            validated++
        }
        check(validated == 1 && BenchmarkFiles.preparedTable(tables, "1".repeat(64), table) == tableInstalled)
        check(Files.readString(tableInstalled.resolve("PREPARED")) == "${"1".repeat(64)}\n")
        // The legacy two-file API must still reject a table-only manifest.
        rejects { BenchmarkFiles.prepared(tables, "1".repeat(64), listOf(table)) }
        rejects { BenchmarkFiles.prepareTable(tableInput, tables, "2".repeat(64), table, { false }, { 32L * 1024 * 1024 + 2 }) {} }
        rejects { BenchmarkFiles.prepareTable(tableInput, tables, "2".repeat(64), table, { false }, { Long.MAX_VALUE }) { throw IOException("wrong selected pack") } }
        rejects { BenchmarkFiles.prepareTable(tableInput, tables, "2".repeat(64), table, { true }, { Long.MAX_VALUE }) {} }
        var tableCancelled = false
        rejects { BenchmarkFiles.prepareTable(tableInput, tables, "2".repeat(64), table, { tableCancelled }, { Long.MAX_VALUE }) { tableCancelled = true } }
        Files.write(tableInput.resolve("titles.bin"), byteArrayOf(9, 2, 3))
        rejects { BenchmarkFiles.prepareTable(tableInput, tables, "2".repeat(64), table, { false }, { Long.MAX_VALUE }) {} }
        for (invalid in listOf(table.copy(name = "pack.zim"), table.copy(name = "../titles.bin"),
            table.copy(bytes = 0), table.copy(bytes = 2L * 1024 * 1024 * 1024 + 1), table.copy(sha256 = "invalid"))) {
            rejects { BenchmarkFiles.prepareTable(tableInput, tables, "2".repeat(64), invalid, { false }, { Long.MAX_VALUE }) {} }
        }
        check(Files.list(tables).use { it.count() } == 1L)
        check(Files.readAllBytes(tableInstalled.resolve("titles.bin")).contentEquals(tableBytes))
        // Re-preparing an existing immutable table still checks its hash and selected-pack compatibility.
        rejects { BenchmarkFiles.prepareTable(tableInput, tables, "1".repeat(64), table, { false }, { Long.MAX_VALUE }) { throw IOException("different pack") } }
        Files.write(tableInstalled.resolve("titles.bin"), byteArrayOf(9, 2, 3))
        rejects { BenchmarkFiles.prepareTable(tableInput, tables, "1".repeat(64), table, { false }, { Long.MAX_VALUE }) {} }
    } finally {
        check(root.toRealPath().parent == Path.of(System.getProperty("java.io.tmpdir")).toRealPath())
        check(root.fileName.toString().startsWith("tonalli-benchmark-test-"))
        Files.walk(root).use { it.sorted(Comparator.reverseOrder()).forEach(Files::delete) }
    }
    println("PASS: exclusive result claims, percentile/priming counts, bounded requests, legacy pair and table-only verified preparation; checksum/cancel/space/native-failure preserve installed data")
}
