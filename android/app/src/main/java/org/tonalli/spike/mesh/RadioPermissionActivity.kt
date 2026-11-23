package org.tonalli.spike.mesh

import android.app.Activity
import android.content.Intent
import android.content.pm.PackageManager
import android.os.Bundle
import android.widget.Toast

class RadioPermissionActivity : Activity() {
    private val permissions = arrayOf(android.Manifest.permission.BLUETOOTH_SCAN, android.Manifest.permission.BLUETOOTH_CONNECT)
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        if (permissions.all { checkSelfPermission(it) == PackageManager.PERMISSION_GRANTED }) startRadio()
        else requestPermissions(permissions, 7210)
    }
    override fun onRequestPermissionsResult(requestCode: Int, requested: Array<out String>, results: IntArray) {
        super.onRequestPermissionsResult(requestCode, requested, results)
        if (requestCode == 7210 && permissions.all { checkSelfPermission(it) == PackageManager.PERMISSION_GRANTED }) startRadio()
        else finish()
    }
    private fun startRadio() {
        try { startForegroundService(Intent(this, RadioService::class.java)) }
        catch (_: Exception) { Toast.makeText(this, "Radio unavailable", Toast.LENGTH_LONG).show() }
        finish()
    }
}
