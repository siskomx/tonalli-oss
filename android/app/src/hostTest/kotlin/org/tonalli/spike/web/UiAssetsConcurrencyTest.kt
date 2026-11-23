package org.tonalli.spike.web

import java.nio.file.Files
import java.nio.file.Path
import java.security.MessageDigest
import java.util.concurrent.CompletableFuture
import java.util.concurrent.CountDownLatch
import java.util.concurrent.Executors
import java.util.concurrent.TimeUnit

fun main() {
    val root = Files.createTempDirectory("tonalli-ui-race-")
    val pool = Executors.newFixedThreadPool(2)
    val firstOpened = CountDownLatch(1)
    val releaseFirst = CountDownLatch(1)
    val secondStarted = CountDownLatch(1)
    val secondOpened = CountDownLatch(1)
    try {
        val previous = Files.createDirectory(root.resolve("ui-${"b".repeat(64)}"))
        Files.writeString(previous.resolve("index.html"), "previous verified UI")
        val content = mapOf("index.html" to "<main>Current UI</main>".toByteArray(), "assets/app.js" to "void 0;".toByteArray())
        val entries = content.map { (name, bytes) ->
            val hash = MessageDigest.getInstance("SHA-256").digest(bytes).joinToString("") { "%02x".format(it) }
            UiAssets.Entry(name, bytes.size.toLong(), hash)
        }
        val first = CompletableFuture.supplyAsync({
            UiAssets.install(root, "a".repeat(64), entries) { name ->
                firstOpened.countDown()
                check(releaseFirst.await(5, TimeUnit.SECONDS))
                checkNotNull(content[name]).inputStream()
            }
        }, pool)
        check(firstOpened.await(5, TimeUnit.SECONDS))
        val second = CompletableFuture.supplyAsync({
            secondStarted.countDown()
            UiAssets.install(root, "a".repeat(64), entries) { name ->
                secondOpened.countDown()
                checkNotNull(content[name]).inputStream()
            }
        }, pool)
        check(secondStarted.await(5, TimeUnit.SECONDS))
        if (secondOpened.await(1, TimeUnit.SECONDS)) second.get(5, TimeUnit.SECONDS)
        releaseFirst.countDown()
        val installed = first.get(5, TimeUnit.SECONDS)
        check(second.get(5, TimeUnit.SECONDS) == installed)
        for ((name, bytes) in content) check(Files.readAllBytes(installed.resolve(name)).contentEquals(bytes))
        check(Files.readString(previous.resolve("index.html")) == "previous verified UI")
        check(Files.list(root).use { paths -> paths.noneMatch { it.fileName.toString().startsWith(".stage-") } })
        println("PASS: concurrent same-version UI launches both reuse verified publication; previous UI is unchanged")
    } finally {
        releaseFirst.countDown()
        pool.shutdownNow()
        check(pool.awaitTermination(5, TimeUnit.SECONDS))
        check(root.toRealPath().parent == Path.of(System.getProperty("java.io.tmpdir")).toRealPath())
        check(root.fileName.toString().startsWith("tonalli-ui-race-"))
        Files.walk(root).use { paths -> paths.sorted(Comparator.reverseOrder()).forEach(Files::deleteIfExists) }
    }
}
