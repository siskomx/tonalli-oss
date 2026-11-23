//! Optional Meshtastic USB terminal spike. No radio code in default builds.
#[cfg(all(target_os = "android", feature = "android-ble"))]
mod android;
#[cfg(all(target_os = "android", feature = "android-ble"))]
static ANDROID_VM: std::sync::OnceLock<jni::JavaVM> = std::sync::OnceLock::new();
#[cfg(all(target_os = "android", feature = "android-ble"))]
pub fn initialize_android(env: &jni::JNIEnv) -> bool {
    let Ok(vm) = env.get_java_vm() else {
        return false;
    };
    let _ = ANDROID_VM.set(vm);
    btleplug::platform::init(env).is_ok()
}
/// Hold this guard for the complete current-thread BLE runtime.
#[cfg(all(target_os = "android", feature = "android-ble"))]
pub fn attach_android_thread() -> Result<jni::AttachGuard<'static>, MeshError> {
    ANDROID_VM
        .get()
        .ok_or(MeshError::Connection)?
        .attach_current_thread()
        .map_err(|_| MeshError::Connection)
}
#[cfg(feature = "serial")]
pub mod config;
#[cfg(feature = "serial")]
pub mod gateway;

#[cfg(feature = "serial")]
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum MeshError {
    #[error("Invalid Mesh configuration; check the documented settings")]
    Configuration,
    #[error("Explicit node handoff is required before connecting")]
    HandoffRequired,
    #[error("The selected radio is already owned by another local gateway")]
    Busy,
    #[error("The selected transport is not included in this build")]
    UnsupportedTransport,
    #[error("The selected radio connection is unavailable")]
    Connection,
    #[error("Radio configuration did not complete in time")]
    ConfigurationTimeout,
    #[error("Radio capabilities are incomplete or exceed configured limits")]
    Capabilities,
    #[error("Accept the detected firmware and select an enabled channel before sending")]
    SendNotAuthorized,
    #[error("The radio connection ended")]
    Disconnected,
    #[error(
        "No matching incoming text was received before the timeout; delivery remains unconfirmed"
    )]
    ReceiveTimeout,
    #[error("Mesh operation stopped")]
    Cancelled,
    #[error("Radio shutdown did not complete cleanly")]
    Shutdown,
}
