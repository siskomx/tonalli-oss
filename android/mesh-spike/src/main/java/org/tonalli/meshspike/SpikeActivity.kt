package org.tonalli.meshspike

import android.Manifest
import android.app.Activity
import android.content.Intent
import android.content.pm.PackageManager
import android.os.Bundle
import android.widget.Button
import android.widget.LinearLayout
import android.widget.TextView

class SpikeActivity : Activity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        val layout = LinearLayout(this).apply { orientation = LinearLayout.VERTICAL; setPadding(24, 24, 24, 24) }
        layout.addView(TextView(this).apply { text = "Mesh terminal spike\nConfigure the selected radio in private app storage. Disconnect other radio clients before starting. No automatic takeover." })
        layout.addView(Button(this).apply {
            text = "Start after radio handoff"
            setOnClickListener {
                val permissions = arrayOf(Manifest.permission.BLUETOOTH_SCAN, Manifest.permission.BLUETOOTH_CONNECT, Manifest.permission.POST_NOTIFICATIONS)
                if (permissions.any { checkSelfPermission(it) != PackageManager.PERMISSION_GRANTED }) {
                    requestPermissions(permissions, 1)
                } else {
                    startForegroundService(Intent(this@SpikeActivity, SpikeService::class.java))
                }
            }
        })
        layout.addView(Button(this).apply {
            text = "Stop"
            setOnClickListener { stopService(Intent(this@SpikeActivity, SpikeService::class.java)) }
        })
        setContentView(layout)
    }
}
