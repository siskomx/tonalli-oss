package org.tonalli.spike.mesh
object MeshNative {
    init { System.loadLibrary("tonalli_core") }
    @JvmStatic external fun initialize(): Boolean
    fun stop() = uniffi.tonalli_core.meshRadioStop()
    fun active() = uniffi.tonalli_core.meshRadioActive()
}
