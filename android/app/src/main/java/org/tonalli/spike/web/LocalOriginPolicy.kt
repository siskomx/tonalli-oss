package org.tonalli.spike.web

import java.net.URI

class LocalOriginPolicy(port: Int) {
    fun allowsGeolocation(origin: String, currentUrl: String?): Boolean {
        val requested = localUri(origin) ?: return false
        val current = currentUrl?.let(::localUri) ?: return false
        return requested.rawPath in setOf("", "/") && requested.rawQuery == null && requested.rawFragment == null && current.rawPath == "/map"
    }
    init { require(port in 1..65535) }
    private val authority = "127.0.0.1:$port"
    private val appPaths = setOf("/", "/launch", "/library", "/article", "/map", "/notes", "/chat", "/content", "/settings", "/files", "/file", "/gallery", "/music", "/videos", "/reader", "/cookbook", "/workshop", "/room", "/assets/project-nomad-NOTICE.txt", "/assets/map/attribution")

    fun allowsMediaExport(currentUrl: String?, id: String): Boolean {
        if (!id.matches(Regex("[a-f0-9]{32}"))) return false
        val current = currentUrl?.let(::localUri) ?: return false
        if (current.rawFragment != null) return false
        return current.rawPath == "/files" || (current.rawPath == "/file" && current.rawQuery == "file=$id")
    }

    fun nativeAction(url: String): String? {
        val uri = localUri(url) ?: return null
        if (uri.rawQuery != null || uri.rawFragment != null) return null
        if (uri.rawPath == "/native/chat-radio") return "chat-radio"
        if (uri.rawPath == "/native/notifications") return "notifications"
        if (uri.rawPath == "/native/hotspot-permission") return "hotspot-permission"
        if (uri.rawPath == "/native/install-update") return "install-update"
        if (uri.rawPath.matches(Regex("/native/export-media/[a-f0-9]{32}"))) return "export-media"
        if (uri.rawPath.matches(Regex("/native/import-media/[a-f0-9]{64}"))) return "import-media"
        return when (uri.rawPath) { "/native/open" -> "open"; "/native/open-map" -> "open-map"; "/native/advanced" -> "advanced"; else -> null }
    }

    fun allowsNotificationState(url: String): Boolean {
        val uri = localUri(url) ?: return false
        return uri.rawQuery == null && uri.rawFragment == null && uri.rawPath == "/native/notifications/state"
    }

    fun allowsNetwork(url: String): Boolean = localUri(url) != null

    fun allowsResource(url: String): Boolean = allowsNetwork(url) || url.startsWith("data:")

    fun allowsNavigation(url: String, mainFrame: Boolean): Boolean {
        val uri = localUri(url) ?: return false
        return !mainFrame || uri.rawPath in appPaths
    }

    private fun localUri(url: String): URI? {
        if (url.any { it <= ' ' || it == '\u007f' || it == '\\' }) return null
        return try {
            URI(url).takeIf { it.scheme == "http" && it.rawAuthority == authority && it.rawUserInfo == null }
        } catch (_: Exception) { null }
    }
}
