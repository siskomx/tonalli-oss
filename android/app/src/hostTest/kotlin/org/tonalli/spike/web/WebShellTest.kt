package org.tonalli.spike.web

import java.io.ByteArrayInputStream
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
    org.tonalli.spike.index.preparationTicketTests()
    mediaFolderTests()
    webLaunchPolicyTests()
    val policy = LocalOriginPolicy(43210)
    check(policy.allowsGeolocation("http://127.0.0.1:43210", "http://127.0.0.1:43210/map"))
    for (url in listOf("http://127.0.0.1:43210/notes", "http://127.0.0.1:43210/article", "http://127.0.0.1:43211/map", "https://example.org/map")) {
        check(!policy.allowsGeolocation("http://127.0.0.1:43210", url))
    }
    for (origin in listOf("http://127.0.0.1:43211", "https://example.org", "http://user@127.0.0.1:43210", "http://127.0.0.1:43210/map", "http://127.0.0.1:43210?x=1")) {
        check(!policy.allowsGeolocation(origin, "http://127.0.0.1:43210/map"))
    }
    check(policy.allowsNavigation("http://127.0.0.1:43210/settings", true))
    check(policy.allowsNavigation("http://127.0.0.1:43210/assets/project-nomad-NOTICE.txt", true))
    check(!policy.allowsNavigation("http://127.0.0.1:43210/assets/unknown.txt", true))
    check(policy.allowsNavigation("http://127.0.0.1:43210/content", true))
    check(policy.nativeAction("http://127.0.0.1:43210/native/open") == "open")
    check(policy.nativeAction("http://127.0.0.1:43210/native/open-map") == "open-map")
    val media = "/native/export-media/" + "a".repeat(32)
    check(policy.nativeAction("http://127.0.0.1:43210$media") == "export-media")
    val mediaImport="/native/import-media/"+"a".repeat(64)
    check(policy.nativeAction("http://127.0.0.1:43210$mediaImport")=="import-media")
    for (url in listOf("http://127.0.0.1:43211$mediaImport","http://127.0.0.1:43210$mediaImport?folder=../notes","http://127.0.0.1:43210$mediaImport#x","http://evil.test$mediaImport"))check(policy.nativeAction(url)==null)
    check(policy.allowsNavigation("http://127.0.0.1:43210/files", true))
    check(policy.nativeAction("http://127.0.0.1:43210/native/notifications") == "notifications")
    check(policy.nativeAction("http://127.0.0.1:43210/native/hotspot-permission") == "hotspot-permission")
    for (url in listOf("http://127.0.0.1:43211/native/hotspot-permission", "http://127.0.0.1:43210/native/hotspot-permission?x=1",
        "http://127.0.0.1:43210/native/hotspot-permission#x", "http://127.0.0.1:43210/native/hotspot-permission/", "https://127.0.0.1:43210/native/hotspot-permission")) {
        check(policy.nativeAction(url) != "hotspot-permission") { "Unexpected hotspot action: $url" }
    }
    check(policy.allowsNotificationState("http://127.0.0.1:43210/native/notifications/state"))
    for (url in listOf("http://127.0.0.1:43211/native/notifications/state", "https://127.0.0.1:43210/native/notifications/state",
        "http://user@127.0.0.1:43210/native/notifications/state", "http://127.0.0.1:43210/native/notifications/state?x=1",
        "http://127.0.0.1:43210/native/notifications/state#x", "http://127.0.0.1:43210/native/notifications/state/",
        "http://127.0.0.1:43210/native/notifications", "http://127.0.0.1:43210/zim/a/native/notifications/state")) {
        check(!policy.allowsNotificationState(url)) { "Unexpected notification state allowance: $url" }
    }
    for (url in listOf("http://127.0.0.1:43211/native/notifications", "http://127.0.0.1:43210/native/notifications?x=1",
        "http://127.0.0.1:43210/native/notifications#x", "http://127.0.0.1:43210/native/notifications/state")) {
        check(policy.nativeAction(url) != "notifications") { "Unexpected notification action: $url" }
    }
    check(policy.nativeAction("http://127.0.0.1:43210/native/install-update") == "install-update")
    for (url in listOf("http://127.0.0.1:43211/native/install-update", "http://127.0.0.1:43210/native/install-update?x=1",
        "http://127.0.0.1:43210/native/install-update#x", "http://127.0.0.1:43210/native/install-update/",
        "https://127.0.0.1:43210/native/install-update", "http://user@127.0.0.1:43210/native/install-update",
        "http://127.0.0.1:43210/zim/a/native/install-update")) {
        check(policy.nativeAction(url) != "install-update") { "Unexpected install-update action: $url" }
    }
val selectedMediaId="a".repeat(32)
check(policy.allowsMediaExport("http://127.0.0.1:43210/files",selectedMediaId))
check(policy.allowsMediaExport("http://127.0.0.1:43210/file?file=$selectedMediaId",selectedMediaId))
for (url in listOf("http://127.0.0.1:43210/article?file=$selectedMediaId", "http://127.0.0.1:43211/file?file=$selectedMediaId",
    "http://127.0.0.1:43210/file?file=${"b".repeat(32)}", "http://127.0.0.1:43210/file?file=$selectedMediaId&extra=1",
    "http://127.0.0.1:43210/%66ile?file=$selectedMediaId", "https://127.0.0.1:43210/file?file=$selectedMediaId")) check(!policy.allowsMediaExport(url,selectedMediaId))
check(!policy.allowsMediaExport(null,selectedMediaId))
check(!policy.allowsMediaExport("http://127.0.0.1:43210/files","../notes.db"))
for (route in listOf("gallery","file","music","videos")) check(policy.allowsNavigation("http://127.0.0.1:43210/$route",true))
    for (url in listOf("http://127.0.0.1:43211$media", "https://example.org$media", "http://user@127.0.0.1:43210$media",
        "http://127.0.0.1:43210$media?path=secret", "http://127.0.0.1:43210$media#x", "http://127.0.0.1:43210/native/export-media/../notes",
        "http://127.0.0.1:43210/native/export-media/short", "http://127.0.0.1:43210/native/export-media/" + "A".repeat(32))) check(policy.nativeAction(url) == null)
    check(policy.allowsNavigation("http://127.0.0.1:43210/assets/map/attribution", true))
    for (url in listOf("http://127.0.0.1:43211/native/open", "http://user@127.0.0.1:43210/native/open", "http://127.0.0.1:43210/native/open?path=x", "http://127.0.0.1:43210/native/open#x", "http://127.0.0.1:43210/zim/a/native/open")) {
        check(policy.nativeAction(url) == null)
    }
    check(policy.allowsNetwork("http://127.0.0.1:43210/api/session"))
    check(policy.allowsNavigation("http://127.0.0.1:43210/article?url=C/Trefoil", true))
    check(policy.allowsNavigation("http://127.0.0.1:43210/notes", true))
    check(policy.allowsNavigation("http://127.0.0.1:43210/chat", true))
    check(!policy.allowsNavigation("http://127.0.0.1:43211/chat", true))
    check(!policy.allowsNavigation("http://localhost:43210/chat", true))
    check(!policy.allowsNavigation("https://127.0.0.1:43210/chat", true))
    check(!policy.allowsNavigation("http://127.0.0.1:43210/chat/anything", true))
    check(!policy.allowsNavigation("http://127.0.0.1:43211/notes", true))
    check(!policy.allowsNavigation("https://127.0.0.1:43210/notes", true))
    check(policy.allowsNavigation("http://127.0.0.1:43210/zim/probe/C/Trefoil", false))
    check(!policy.allowsNavigation("http://127.0.0.1:43210/zim/probe/C/Trefoil", true))
    for (url in listOf("https://127.0.0.1:43210/", "http://localhost:43210/", "http://127.0.0.1:43211/",
        "http://127.0.0.1/", "http://127.0.0.1:43210@evil.test/", "http://user@127.0.0.1:43210/",
        "http://127.0.0.1.evil.test:43210/", "http://2130706433:43210/", "http://[::1]:43210/",
        "http://127.0.0.1:43210/\\evil", " http://127.0.0.1:43210/", "file:///secret", "content://notes",
        "intent://anything", "javascript:alert(1)", "data:text/html,hello", "blob:http://evil.test/id")) {
        check(!policy.allowsNetwork(url)) { "Unexpected network allowance: $url" }
        check(!policy.allowsNavigation(url, true)) { "Unexpected navigation allowance: $url" }
    }
    check(policy.allowsResource("data:image/png;base64,AAAA"))
    check(!policy.allowsNavigation("data:text/html,hello", false))

    var closes = 0
    val late = WorkerSession<AutoCloseable>()
    late.requestClose()
    check(!late.adopt(AutoCloseable { closes++ }))
    late.release()
    check(closes == 1)
    val active = WorkerSession<AutoCloseable>()
    check(active.adopt(AutoCloseable { closes++ }))
    active.requestClose()
    active.release()
    active.release()
    check(closes == 2)

    val root = Files.createTempDirectory("tonalli-ui-test-")
    try {
        val payload = mapOf("index.html" to "trusted page".toByteArray(), "fonts/Noto Sans @1/data.pbf" to byteArrayOf(1, 2, 3))
        val records = payload.map { (path, bytes) -> UiAssets.Entry(path, bytes.size.toLong(), digest(bytes)) }
        val installed = UiAssets.install(root, "a".repeat(64), records) { ByteArrayInputStream(payload.getValue(it)) }
        check(Files.readAllBytes(installed.resolve("index.html")).contentEquals(payload.getValue("index.html")))
        check(UiAssets.install(root, "a".repeat(64), records) { error("Installed payload should be reused") } == installed)
        rejects { UiAssets.install(root, "b".repeat(64), records) { ByteArrayInputStream(ByteArray(payload.getValue(it).size)) } }
        check(!Files.exists(root.resolve("ui-" + "b".repeat(64))))
        check(Files.exists(installed))
        for (path in listOf("../escape", "/absolute", "a//b", "a/./b", "a/../b", "C:/root", "a\\b")) {
            rejects { UiAssets.install(root, "c".repeat(64), records + UiAssets.Entry(path, 0, digest(byteArrayOf()))) { ByteArrayInputStream(byteArrayOf()) } }
        }
        rejects { UiAssets.install(root, "d".repeat(64), records.map { it.copy(bytes = Long.MAX_VALUE) }) { ByteArrayInputStream(byteArrayOf()) } }
        rejects { UiAssets.readManifest(ByteArrayInputStream(ByteArray(512 * 1024 + 1))) }
        Files.writeString(installed.resolve("unlisted.txt"), "not trusted")
        rejects { UiAssets.install(root, "a".repeat(64), records) { error("No copy") } }
        check(Files.list(root).use { paths -> paths.noneMatch { it.fileName.toString().startsWith(".stage-") } })
    } finally {
        check(root.toRealPath().parent == Path.of(System.getProperty("java.io.tmpdir")).toRealPath())
        check(root.fileName.toString().startsWith("tonalli-ui-test-"))
        Files.walk(root).use { paths -> paths.sorted(Comparator.reverseOrder()).forEach(Files::delete) }
    }
    println("PASS: exact loopback policy, forbidden navigation, inline resource distinction, late session cleanup, verified nested UI staging/reuse/corruption/path/size/unlisted checks")
}
