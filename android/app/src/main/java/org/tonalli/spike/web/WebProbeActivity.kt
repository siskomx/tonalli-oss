package org.tonalli.spike.web

import android.annotation.SuppressLint
import android.app.Activity
import android.content.Intent
import android.content.pm.ApplicationInfo
import android.net.http.SslError
import android.os.Bundle
import android.util.Log
import android.webkit.CookieManager
import android.webkit.GeolocationPermissions
import android.webkit.PermissionRequest
import android.webkit.RenderProcessGoneDetail
import android.webkit.ServiceWorkerClient
import android.webkit.ServiceWorkerController
import android.webkit.SslErrorHandler
import android.webkit.ValueCallback
import android.webkit.WebChromeClient
import android.webkit.WebResourceError
import android.webkit.WebResourceRequest
import android.webkit.WebResourceResponse
import android.webkit.WebSettings
import android.webkit.WebView
import android.webkit.WebViewClient
import android.widget.Button
import android.widget.LinearLayout
import android.widget.TextView
import java.io.ByteArrayInputStream
import java.util.concurrent.Executors
import java.util.concurrent.RejectedExecutionException
import uniffi.tonalli_core.LoopbackInfo
import org.tonalli.spike.benchmark.DeferredWebLaunch
import org.tonalli.spike.benchmark.WebBenchmarkRequest

/** No native bridge: the shared UI talks to Rust using ordinary loopback HTTP. */
class WebProbeActivity : Activity() {
    private val worker = Executors.newSingleThreadExecutor()
    private val aiCanceller = Executors.newSingleThreadExecutor()
    @Volatile private var startingSession: LoopbackSession? = null
    private var aiRequested = false
    private var replacingOwner = false
    private val lifetime = WorkerSession<LoopbackSession>()
    @Volatile private var policy: LocalOriginPolicy? = null
    private var webView: WebView? = null
    private lateinit var column: LinearLayout
    private lateinit var status: TextView
    private var benchmark: WebBenchmarkRequest? = null
    private val deferred = DeferredWebLaunch()
    private var launchInfo: LoopbackInfo? = null
    private var bootstrapping = false
    private var redirectedEntry = false
    private var reader = false
    private var locationRequest: Pair<String, GeolocationPermissions.Callback>? = null
    private var exportId: String? = null
    private var readerDestination = "library"

    override fun onCreate(state: Bundle?) {
        super.onCreate(state)
        aiRequested = intent.action == OPEN_AI
        reader = intent.action == OPEN_READER
        if (reader && state?.getString("reader_destination") == "files") readerDestination = "files"
        exportId = state?.getString("media_export_id")?.takeIf { it.matches(Regex("[a-f0-9]{32}")) }
        if (reader) org.tonalli.spike.lan.NativeSharing.bindWindow(window)
        status = TextView(this).apply {
            text = if (reader) getString(org.tonalli.spike.R.string.reader_starting) else "Starting private loopback UI…"
            setPadding(16, 8, 16, 8)
        }
        column = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            if (!reader) addView(Button(this@WebProbeActivity).apply {
                if (reader) setText(org.tonalli.spike.R.string.reader_done) else text = "Close shared UI"
                setOnClickListener { if (reader) finishAndRemoveTask() else finish() }
            })
            addView(status)
        }
        setContentView(column)
        if (intent.action in setOf(WebBenchmarkRequest.ACTION, WebBenchmarkRequest.LOAD)) {
            if ((applicationInfo.flags and ApplicationInfo.FLAG_DEBUGGABLE) == 0 || intent.action != WebBenchmarkRequest.ACTION) {
                status.text = "No fresh debug WebView benchmark preparation; restart the measured attempt."
                return
            }
            benchmark = runCatching { WebBenchmarkRequest.checked(intent.getStringExtra("dataset_id"),
                intent.getStringExtra("manifest_sha256"), intent.getStringExtra("profile")) }.getOrElse {
                status.text = "Invalid WebView benchmark request."
                return
            }
        }
        worker.execute {
            try {
                val session = LoopbackSession.start(applicationContext, benchmark, aiRequested,
                    reader = reader,
                    cancelled = { lifetime.isClosing }, onStarting = { startingSession = it })
                if (lifetime.adopt(session)) runOnUiThread {
                    if (!lifetime.isClosing) {
                        try {
                            showUi(session.info)
                            session.aiOffReason?.let { status.text = it }
                            session.readerNotice?.let { status.text = it; status.visibility = android.view.View.VISIBLE }
                        } catch (error: Throwable) {
                            destroyWebView()
                            showFailure(error)
                            stopServer()
                        }
                    }
                }
            } catch (error: Throwable) {
                runOnUiThread { if (!lifetime.isClosing) { showFailure(error); stopServer() } }
            }
        }
    }

    @SuppressLint("SetJavaScriptEnabled") // Required by the trusted packaged React UI; no native bridge exists.
    private fun showUi(info: LoopbackInfo) {
        policy = LocalOriginPolicy(info.port.toInt())
        val view = WebView(this)
        webView = view
        WebView.setWebContentsDebuggingEnabled((applicationInfo.flags and ApplicationInfo.FLAG_DEBUGGABLE) != 0)
        view.settings.apply {
            if (reader) userAgentString += " TonalliNative/Android"
            javaScriptEnabled = true
            domStorageEnabled = true
            allowFileAccess = false
            allowContentAccess = false
            allowFileAccessFromFileURLs = false
            allowUniversalAccessFromFileURLs = false
            mixedContentMode = WebSettings.MIXED_CONTENT_NEVER_ALLOW
            javaScriptCanOpenWindowsAutomatically = false
            setSupportMultipleWindows(false)
            setGeolocationEnabled(reader)
            mediaPlaybackRequiresUserGesture = true
            if (reader) {
                setSupportZoom(true)
                builtInZoomControls = true
                displayZoomControls = false
            }
            // Only our loopback server is browsed; do not perform online reputation lookups.
            safeBrowsingEnabled = false
        }
        CookieManager.getInstance().apply {
            setAcceptCookie(true)
            setAcceptThirdPartyCookies(view, false)
        }
        ServiceWorkerController.getInstance().apply {
            // The M1 stub has no service worker; ordinary MapLibre workers are unaffected.
            serviceWorkerWebSettings.apply { allowFileAccess = false; allowContentAccess = false; blockNetworkLoads = true }
            setServiceWorkerClient(stoppedWorkers)
        }
        view.webChromeClient = object : WebChromeClient() {
            override fun onPermissionRequest(request: PermissionRequest) { request.deny() }
            override fun onGeolocationPermissionsShowPrompt(origin: String, callback: GeolocationPermissions.Callback) {
                if (!reader || lifetime.isClosing || policy?.allowsGeolocation(origin, webView?.url) != true) {
                    callback.invoke(origin, false, false)
                } else if (hasLocationPermission()) {
                    callback.invoke(origin, true, false)
                } else {
                    clearLocationRequest()
                    locationRequest = origin to callback
                    requestPermissions(arrayOf(android.Manifest.permission.ACCESS_FINE_LOCATION, android.Manifest.permission.ACCESS_COARSE_LOCATION), 7104)
                }
            }
            override fun onGeolocationPermissionsHidePrompt() { clearLocationRequest() }
            override fun onCreateWindow(view: WebView, dialog: Boolean, gesture: Boolean, result: android.os.Message) = false
            override fun onShowFileChooser(view: WebView, callback: ValueCallback<Array<android.net.Uri>>, params: FileChooserParams): Boolean {
                callback.onReceiveValue(null)
                return true
            }
        }
        view.setDownloadListener { _, _, _, _, _ ->
            status.visibility = android.view.View.VISIBLE
            status.text = if (reader) getString(org.tonalli.spike.R.string.reader_file_blocked) else "Downloads are disabled in this M1 WebView probe."
        }
        view.webViewClient = object : WebViewClient() {
            override fun shouldOverrideUrlLoading(view: WebView, request: WebResourceRequest): Boolean {
                val url = request.url.toString()
                if (reader && request.isForMainFrame && request.hasGesture() && !lifetime.isClosing) {
                    when (policy?.nativeAction(url)) {
                        "chat-radio" -> { if (android.net.Uri.parse(view.url ?: "").path in setOf("/room", "/settings")) startActivity(Intent(this@WebProbeActivity, org.tonalli.spike.mesh.RadioPermissionActivity::class.java)); return true }
                        "notifications" -> { recoverNotifications(); return true }
                        "hotspot-permission" -> { allowHotspot(); return true }
                        "install-update" -> { if (android.net.Uri.parse(view.url ?: "").path == "/settings") installUpdate(); return true }
                        "open" -> { startActivityForResult(Intent(this@WebProbeActivity, org.tonalli.spike.reader.OpenContentActivity::class.java), OPEN_CONTENT); return true }
                        "open-map" -> { startActivityForResult(Intent(this@WebProbeActivity, org.tonalli.spike.downloads.OpenMapActivity::class.java), OPEN_MAP); return true }
                        "advanced" -> { showSettings(); return true }
                        "export-media" -> {
                            val id = android.net.Uri.parse(url).lastPathSegment ?: ""
                            if (policy?.allowsMediaExport(view.url, id) == true) saveMedia(id)
                            return true
                        }
                        "import-media" -> {
                            if (android.net.Uri.parse(view.url ?: "").path == "/files") startActivity(Intent(this@WebProbeActivity, org.tonalli.spike.media.MediaImportActivity::class.java)
                                .putExtra("ticket", android.net.Uri.parse(url).lastPathSegment))
                            return true
                        }
                    }
                }
                if (((benchmark != null && bootstrapping) || aiRequested || reader) && !redirectedEntry && request.isForMainFrame &&
                    url == "http://127.0.0.1:${info.port}/") {
                    redirectedEntry = true
                    view.loadUrl("http://127.0.0.1:${info.port}/" + if (aiRequested) "chat" else if (reader) readerDestination else "library?pack=probe")
                    return true
                }
                return lifetime.isClosing || policy?.allowsNavigation(url, request.isForMainFrame) != true
            }

            override fun shouldInterceptRequest(view: WebView, request: WebResourceRequest): WebResourceResponse? = intercept(request)

            override fun onReceivedSslError(view: WebView, handler: SslErrorHandler, error: SslError) { handler.cancel() }

            override fun onReceivedError(view: WebView, request: WebResourceRequest, error: WebResourceError) {
                if (request.isForMainFrame) {
                    status.visibility = android.view.View.VISIBLE
                    status.text = if (reader) getString(org.tonalli.spike.R.string.reader_open_failed)
                        else "WebView request failed (${error.errorCode}); readiness is unverified."
                }
            }

            override fun onReceivedHttpError(view: WebView, request: WebResourceRequest, response: WebResourceResponse) {
                if (request.isForMainFrame) {
                    status.visibility = android.view.View.VISIBLE
                    status.text = if (reader) getString(org.tonalli.spike.R.string.reader_open_failed)
                        else "WebView HTTP ${response.statusCode}; readiness is unverified."
                }
            }

            override fun onRenderProcessGone(view: WebView, detail: RenderProcessGoneDetail): Boolean {
                destroyWebView()
                status.visibility = android.view.View.VISIBLE
                status.text = if (reader) getString(org.tonalli.spike.R.string.reader_open_failed) else "WebView renderer stopped; close this screen and retry."
                stopServer()
                return true
            }
        }
        column.addView(view, LinearLayout.LayoutParams(LinearLayout.LayoutParams.MATCH_PARENT, 0, 1f))
        status.text = if (reader) getString(org.tonalli.spike.R.string.reader_private) else "127.0.0.1:${info.port} — device rendering/readiness gates pending"
        if (reader) status.visibility = android.view.View.GONE
        Log.i("TonalliM1", "Shared UI loopback port=${info.port}; provider=${WebView.getCurrentWebViewPackage()?.versionName}")
        launchInfo = info
        if (benchmark == null) view.loadUrl(info.launchUrl)
        else {
            view.loadUrl("about:blank")
            deferred.ready()
            status.text = "Prepared benchmark WebView; waiting for explicit LOAD."
            releaseDeferred()
        }
    }

    private fun intercept(request: WebResourceRequest): WebResourceResponse? {
        val current = if (lifetime.isClosing) null else policy
        val url = request.url.toString()
        if (benchmark != null && !bootstrapping && url == "about:blank" && request.isForMainFrame) return null
        if (reader && current != null && !request.isForMainFrame && request.method == "GET" && current.allowsNotificationState(url)) {
            val body = """{"enabled":${notificationsEnabled()}}""".toByteArray()
            return WebResourceResponse("application/json", "utf-8", 200, "OK", mapOf("Cache-Control" to "no-store"), ByteArrayInputStream(body))
        }
        if (current != null && current.allowsResource(url) &&
            (!request.isForMainFrame || current.allowsNavigation(url, true))) return null
        return WebResourceResponse("text/plain", "utf-8", 403, "Blocked", mapOf("Cache-Control" to "no-store"), ByteArrayInputStream(byteArrayOf()))
    }

    private fun showFailure(error: Throwable) {
        status.visibility = android.view.View.VISIBLE
        status.text = if (reader) getString(org.tonalli.spike.R.string.reader_open_failed)
            else "Shared UI startup failed: ${error.javaClass.simpleName}: ${error.message}"
        Log.e("TonalliM1", "Shared UI startup failed: ${error.javaClass.simpleName}")
    }

    private fun hasLocationPermission() = listOf(android.Manifest.permission.ACCESS_FINE_LOCATION, android.Manifest.permission.ACCESS_COARSE_LOCATION)
        .any { checkSelfPermission(it) == android.content.pm.PackageManager.PERMISSION_GRANTED }

    private fun notificationsEnabled() = getSystemService(android.app.NotificationManager::class.java)?.areNotificationsEnabled() == true

    // The system dialog is offered once; afterwards Android may silently refuse, so send the user to the app's notification settings.
    private fun recoverNotifications() {
        if (notificationsEnabled()) return
        val asked = getSharedPreferences("notifications", MODE_PRIVATE)
        if (android.os.Build.VERSION.SDK_INT >= 33 && !asked.getBoolean("asked", false) &&
            checkSelfPermission(android.Manifest.permission.POST_NOTIFICATIONS) != android.content.pm.PackageManager.PERMISSION_GRANTED) {
            asked.edit().putBoolean("asked", true).apply()
            requestPermissions(arrayOf(android.Manifest.permission.POST_NOTIFICATIONS), 7105)
            return
        }
        try { startActivity(Intent(android.provider.Settings.ACTION_APP_NOTIFICATION_SETTINGS).putExtra(android.provider.Settings.EXTRA_APP_PACKAGE, packageName)) }
        catch (_: Exception) {
            try { startActivity(Intent(android.provider.Settings.ACTION_APPLICATION_DETAILS_SETTINGS, android.net.Uri.fromParts("package", packageName, null))) }
            catch (_: Exception) { android.widget.Toast.makeText(this, org.tonalli.spike.R.string.notifications_settings_failed, android.widget.Toast.LENGTH_LONG).show() }
        }
    }

    // Android shows its own "Install update?" prompt; Core re-verifies the package first and again as the installer reads it.
    private fun installUpdate() {
        fun say(text: Int) = android.widget.Toast.makeText(this, text, android.widget.Toast.LENGTH_LONG).show()
        if (!packageManager.canRequestPackageInstalls()) {
            say(org.tonalli.spike.R.string.update_install_allow)
            try { startActivity(Intent(android.provider.Settings.ACTION_MANAGE_UNKNOWN_APP_SOURCES, android.net.Uri.parse("package:$packageName"))) }
            catch (_: Exception) { say(org.tonalli.spike.R.string.update_install_failed) }
            return
        }
        try {
            worker.execute {
                val ready = try { uniffi.tonalli_core.updateReady(java.io.File(filesDir, "core").absolutePath) } catch (_: Exception) { null }
                runOnUiThread {
                    if (lifetime.isClosing) return@runOnUiThread
                    if (ready == null) { say(org.tonalli.spike.R.string.update_install_unavailable); return@runOnUiThread }
                    try {
                        startActivity(Intent(Intent.ACTION_VIEW)
                            .setDataAndType(org.tonalli.spike.update.UpdateProvider.uri(this), org.tonalli.spike.update.UpdateProvider.APK_MIME)
                            .addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION))
                    } catch (_: Exception) { say(org.tonalli.spike.R.string.update_install_failed) }
                }
            }
        } catch (_: RejectedExecutionException) { say(org.tonalli.spike.R.string.update_install_failed) }
    }

    // Same one-time dialog policy as notifications: the sharing list refreshes itself once Android reports the grant.
    private fun allowHotspot() {
        val permission = android.Manifest.permission.NEARBY_WIFI_DEVICES
        if (checkSelfPermission(permission) == android.content.pm.PackageManager.PERMISSION_GRANTED) return
        val asked = getSharedPreferences("notifications", MODE_PRIVATE)
        if (!asked.getBoolean("hotspot_asked", false)) {
            asked.edit().putBoolean("hotspot_asked", true).apply()
            requestPermissions(arrayOf(permission), 7106)
            return
        }
        try { startActivity(Intent(android.provider.Settings.ACTION_APPLICATION_DETAILS_SETTINGS, android.net.Uri.fromParts("package", packageName, null))) }
        catch (_: Exception) { android.widget.Toast.makeText(this, org.tonalli.spike.R.string.notifications_settings_failed, android.widget.Toast.LENGTH_LONG).show() }
    }

    private fun clearLocationRequest() {
        val pending = locationRequest
        locationRequest = null
        pending?.second?.invoke(pending.first, false, false)
    }

    override fun onRequestPermissionsResult(requestCode: Int, permissions: Array<out String>, grantResults: IntArray) {
        super.onRequestPermissionsResult(requestCode, permissions, grantResults)
        if (requestCode != 7104) return
        val pending = locationRequest ?: return
        locationRequest = null
        val allowed = reader && !lifetime.isClosing && hasLocationPermission() && policy?.allowsGeolocation(pending.first, webView?.url) == true
        pending.second.invoke(pending.first, allowed, false)
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        if (WebLaunchPolicy.recreateForIntent(intent.action, WebBenchmarkRequest.LOAD)) {
            setIntent(intent)
            // Explicit recreate is not reliably reported by isChangingConfigurations.
            replacingOwner = true
            recreate()
            return
        }
        if (intent.action != WebBenchmarkRequest.LOAD || benchmark == null || lifetime.isClosing) return
        val request = runCatching { WebBenchmarkRequest.checked(intent.getStringExtra("dataset_id"),
            intent.getStringExtra("manifest_sha256"), intent.getStringExtra("profile")) }.getOrNull()
        if (request != benchmark) { status.text = "WebView benchmark profile/input mismatch rejected."; return }
        deferred.request()
        releaseDeferred()
    }

    @Deprecated("Native content picker callback")
    override fun onActivityResult(requestCode: Int, resultCode: Int, data: Intent?) {
        super.onActivityResult(requestCode, resultCode, data)
        if (requestCode == EXPORT_MEDIA) {
            val id = exportId; exportId = null
            val uri = data?.data
            if (id != null && resultCode == RESULT_OK && uri?.scheme == "content" && !lifetime.isClosing) {
                worker.execute {
                    val result = runCatching {
                        val descriptor = contentResolver.openFileDescriptor(uri, "w") ?: throw java.io.IOException("Output unavailable")
                        val fd = descriptor.use { it.detachFd() }
                        uniffi.tonalli_core.mediaExportFd(java.io.File(filesDir, "core").absolutePath, id, fd)
                    }
                    runOnUiThread { if (!lifetime.isClosing) android.widget.Toast.makeText(this, if (result.isSuccess) org.tonalli.spike.R.string.media_saved else org.tonalli.spike.R.string.media_save_failed, android.widget.Toast.LENGTH_LONG).show() }
                }
            }
            return
        }
        if (requestCode == OPEN_MAP && resultCode == RESULT_OK && reader && !lifetime.isClosing) {
            webView?.reload()
            return
        }
        if (requestCode in setOf(OPEN_CONTENT, SETUP_INDEX) && resultCode == RESULT_OK) {
            replacingOwner = true
            recreate()
        }
    }

    private fun showSettings() {
        android.app.AlertDialog.Builder(this).setTitle(org.tonalli.spike.R.string.reader_advanced)
                    .setItems(arrayOf(getString(org.tonalli.spike.R.string.index_setup), getString(org.tonalli.spike.R.string.reader_diagnostics))) { _, which ->
                        if (which == 0) startActivityForResult(Intent(this, org.tonalli.spike.reader.ReaderIndexActivity::class.java), SETUP_INDEX)
                        else startActivity(Intent(this, org.tonalli.spike.MainActivity::class.java).setAction("org.tonalli.spike.DIAGNOSTICS"))
                    }.setNegativeButton(android.R.string.cancel, null).show()
    }

    private fun saveMedia(id: String) {
        if (exportId != null || !id.matches(Regex("[a-f0-9]{32}"))) return
        exportId = id
        worker.execute {
            val result = runCatching { uniffi.tonalli_core.mediaFileInfo(java.io.File(filesDir, "core").absolutePath, id) }
            runOnUiThread {
                if (lifetime.isClosing) { exportId = null; return@runOnUiThread }
                result.onSuccess { file ->
                    try { startActivityForResult(Intent(Intent.ACTION_CREATE_DOCUMENT).addCategory(Intent.CATEGORY_OPENABLE).setType("application/octet-stream")
                        .putExtra(Intent.EXTRA_TITLE, file.name), EXPORT_MEDIA) }
                    catch (_: Exception) { exportId = null; android.widget.Toast.makeText(this, org.tonalli.spike.R.string.media_save_failed, android.widget.Toast.LENGTH_LONG).show() }
                }.onFailure { exportId = null; android.widget.Toast.makeText(this, org.tonalli.spike.R.string.media_save_failed, android.widget.Toast.LENGTH_LONG).show() }
            }
        }
    }

    override fun onSaveInstanceState(outState: Bundle) {
        outState.putString("reader_destination", if (android.net.Uri.parse(webView?.url ?: "").path == "/files") "files" else "library")
        exportId?.let { outState.putString("media_export_id", it) }
        super.onSaveInstanceState(outState)
    }

    @Deprecated("Platform back closes the normal reader task")
    override fun onBackPressed() {
        val inLibrary = webView?.url?.let { android.net.Uri.parse(it).path == "/library" } == true
        if (reader && !inLibrary && webView?.canGoBack() == true) webView?.goBack()
        else if (reader) finishAndRemoveTask() else super.onBackPressed()
    }

    private fun releaseDeferred() {
        val view = webView ?: return
        val info = launchInfo ?: return
        if (deferred.takeLaunch() && !lifetime.isClosing) {
            bootstrapping = true
            status.text = "Loading prepared benchmark library; rendered readiness remains to be observed."
            view.loadUrl(info.launchUrl)
        }
    }

    private fun stopServer() {
        if (lifetime.isClosing) return
        lifetime.requestClose()
        val captured = startingSession
        try { aiCanceller.execute { runCatching { captured?.requestAiStop() }.onFailure { Log.e("TonalliM1", "Shared AI detach failed: ${it.javaClass.simpleName}") } } }
        catch (_: RejectedExecutionException) { /* Shutdown follows owned cleanup. */ }
        policy = null
        worker.execute {
            runCatching { lifetime.release() }.onFailure { Log.e("TonalliM1", "Loopback cleanup failed: ${it.javaClass.simpleName}") }
            startingSession = null
            aiCanceller.shutdown()
        }
        worker.shutdown()
    }

    override fun onResume() { super.onResume(); webView?.onResume() }
    override fun onPause() {
        webView?.onPause()
        if (aiRequested) {
            status.text = "Shared AI closed on departure. Return to Main to explicitly reopen; ordinary offline UI remains available."
            stopServer()
            if (WebLaunchPolicy.finishOnPause(aiRequested, replacingOwner, isChangingConfigurations)) finish()
        }
        super.onPause()
    }

    override fun onDestroy() {
        exportId = null
        clearLocationRequest()
        if (!aiRequested && isFinishing && !isChangingConfigurations) org.tonalli.spike.tv.NativeTv.stopForExit()
        stopServer()
        destroyWebView()
        super.onDestroy()
    }

    private fun destroyWebView() {
        val view = webView ?: return
        webView = null
        runCatching {
            try { view.stopLoading(); column.removeView(view); view.removeAllViews() } finally { view.destroy() }
        }.onFailure { Log.e("TonalliM1", "WebView cleanup failed: ${it.javaClass.simpleName}") }
    }

    companion object {
        const val OPEN_AI = WebLaunchPolicy.OPEN_AI
        const val OPEN_READER = WebLaunchPolicy.OPEN_READER
        private const val OPEN_CONTENT = 82
        private const val OPEN_MAP = 85
        private const val SETUP_INDEX = 84
        private const val EXPORT_MEDIA = 87
        private val stoppedWorkers = object : ServiceWorkerClient() {
            override fun shouldInterceptRequest(request: WebResourceRequest) =
                WebResourceResponse("text/plain", "utf-8", 403, "Disabled", emptyMap(), ByteArrayInputStream(byteArrayOf()))
        }
    }
}
