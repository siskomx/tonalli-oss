package org.tonalli.spike.web

import android.content.Context
import java.io.File
import java.io.IOException
import java.nio.file.Path
import org.json.JSONObject

internal fun installPackagedUi(context: Context): Path {
    val bytes = context.assets.open("ui-manifest.json").use(UiAssets::readManifest)
    val manifest = JSONObject(bytes.toString(Charsets.UTF_8))
    if (manifest.getInt("version") != 1) throw IOException("Unsupported packaged UI version")
    val records = manifest.getJSONArray("files")
    if (records.length() !in 1..2000) throw IOException("Invalid packaged UI file count")
    val entries = (0 until records.length()).map { position ->
        val entry = records.getJSONObject(position)
        UiAssets.Entry(entry.getString("path"), entry.getLong("bytes"), entry.getString("sha256"))
    }
    return UiAssets.install(File(context.filesDir, "web-ui").toPath(), manifest.getString("id"), entries) {
        context.assets.open("ui/$it")
    }
}
