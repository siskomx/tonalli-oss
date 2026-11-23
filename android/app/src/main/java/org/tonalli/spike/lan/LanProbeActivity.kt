package org.tonalli.spike.lan

import android.Manifest
import android.app.Activity
import android.content.*
import android.content.pm.PackageManager
import android.os.*
import android.view.WindowManager
import android.widget.*

/** Disposable M1 service controls; pairing secrets stay on this private screen. */
class LanProbeActivity : Activity() {
    private var controls: LanService.Controls? = null
    private var bound = false
    private var pendingHotspot: Boolean? = null
    private var actionFailure: String? = null
    private lateinit var output: TextView
    private lateinit var wake: CheckBox
    private lateinit var deviceId: EditText
    private val handler = Handler(Looper.getMainLooper())
    private val refresh = object : Runnable {
        override fun run() {
            controls?.let {
                val next = actionFailure ?: it.status()
                if (output.text.toString() != next) output.text = next
            }
            handler.postDelayed(this, 500)
        }
    }
    private val connection = object : ServiceConnection {
        override fun onServiceConnected(name: ComponentName, binder: IBinder) { controls = binder as LanService.Controls }
        override fun onServiceDisconnected(name: ComponentName) { controls = null }
    }

    override fun onCreate(state: Bundle?) {
        super.onCreate(state)
        window.addFlags(WindowManager.LayoutParams.FLAG_SECURE)
        output = TextView(this).apply { setTextIsSelectable(true); text = "LAN server is off. Start explicitly to share installed packs." }
        wake = CheckBox(this).apply { text = "Keep CPU awake (30 minute maximum)" }
        deviceId = EditText(this).apply { hint = "Device ID to kick" }
        val column = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            addView(wake)
            addView(button("Start on Wi-Fi") { requestStart(false) })
            addView(button("Start local-only hotspot") { requestStart(true) })
            addView(button("Stop LAN sharing") {
                actionFailure = null
                try { startService(Intent(this@LanProbeActivity, LanService::class.java).setAction(LanService.STOP)) }
                catch (_: RuntimeException) { showFailure("Android refused the Stop request; retry while the app is visible") }
            })
            addView(button("Refresh pairing code and devices") { controls?.refresh() })
            addView(deviceId)
            addView(button("Kick device") { controls?.kick(deviceId.text.toString()) })
            addView(output)
        }
        setContentView(ScrollView(this).apply { addView(column) })
    }

    private fun button(label: String, action: () -> Unit) = Button(this).apply { text = label; setOnClickListener { action() } }

    private fun requestStart(hotspot: Boolean) {
        actionFailure = null
        pendingHotspot = hotspot
        val permissions = mutableListOf<String>()
        if (checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED)
            permissions += Manifest.permission.POST_NOTIFICATIONS
        if (hotspot && checkSelfPermission(Manifest.permission.NEARBY_WIFI_DEVICES) != PackageManager.PERMISSION_GRANTED)
            permissions += Manifest.permission.NEARBY_WIFI_DEVICES
        if (permissions.isEmpty()) startPending() else {
            try { requestPermissions(permissions.toTypedArray(), 10) }
            catch (_: RuntimeException) { pendingHotspot = null; showFailure("Android refused the permission request") }
        }
    }

    private fun startPending() {
        val hotspot = pendingHotspot ?: return
        pendingHotspot = null
        if (hotspot && checkSelfPermission(Manifest.permission.NEARBY_WIFI_DEVICES) != PackageManager.PERMISSION_GRANTED) {
            showFailure("Nearby Wi-Fi permission is required for the local-only hotspot"); return
        }
        try {
            startForegroundService(Intent(this, LanService::class.java).setAction(LanService.START)
                .putExtra("hotspot", hotspot).putExtra("wake", wake.isChecked))
        } catch (_: RuntimeException) {
            showFailure("Android refused foreground sharing; check permissions and retry while the app is visible")
        }
    }

    private fun showFailure(message: String) { actionFailure = message; output.text = message }

    override fun onRequestPermissionsResult(code: Int, permissions: Array<out String>, grants: IntArray) {
        super.onRequestPermissionsResult(code, permissions, grants)
        if (code == 10) startPending()
    }

    override fun onStart() {
        super.onStart()
        bound = bindService(Intent(this, LanService::class.java), connection, BIND_AUTO_CREATE)
        handler.post(refresh)
    }
    override fun onStop() {
        handler.removeCallbacks(refresh)
        if (bound) unbindService(connection)
        bound = false
        controls = null
        super.onStop()
    }
}
