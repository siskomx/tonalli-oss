package org.tonalli.meshspike

object MeshNative {
    init { System.loadLibrary("tonalli_mesh_gateway") }
    @JvmStatic external fun initialize(): Boolean
    @JvmStatic external fun run(configuration: String, handoffConfirmed: Boolean): String
    @JvmStatic external fun stop()
}
