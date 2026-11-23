use crate::{config::Config, gateway, MeshError};
use btleplug::api::{Central, Manager as _, Peripheral as _};
use jni::{
    objects::{JClass, JString},
    sys::{jboolean, jstring},
    JNIEnv,
};
use std::{
    panic::{catch_unwind, AssertUnwindSafe},
    sync::{Mutex, Once},
};
use tokio_util::sync::CancellationToken;

async fn release_ble(selector: &str) -> Result<(), MeshError> {
    let manager = btleplug::platform::Manager::new()
        .await
        .map_err(|_| MeshError::Shutdown)?;
    for adapter in manager.adapters().await.map_err(|_| MeshError::Shutdown)? {
        adapter.stop_scan().await.map_err(|_| MeshError::Shutdown)?;
        for peripheral in adapter
            .peripherals()
            .await
            .map_err(|_| MeshError::Shutdown)?
        {
            if peripheral
                .address()
                .to_string()
                .eq_ignore_ascii_case(selector)
                && peripheral
                    .is_connected()
                    .await
                    .map_err(|_| MeshError::Shutdown)?
            {
                peripheral
                    .disconnect()
                    .await
                    .map_err(|_| MeshError::Shutdown)?;
            }
        }
    }
    Ok(())
}

static RUN: Mutex<Option<CancellationToken>> = Mutex::new(None);
static PANIC_POLICY: Once = Once::new();
struct RunGuard;
impl Drop for RunGuard {
    fn drop(&mut self) {
        if let Ok(mut run) = RUN.lock() {
            *run = None;
        }
    }
}

#[no_mangle]
pub extern "system" fn Java_org_tonalli_meshspike_MeshNative_initialize(
    env: JNIEnv,
    _class: JClass,
) -> jboolean {
    PANIC_POLICY.call_once(|| std::panic::set_hook(Box::new(|_| eprintln!("Mesh worker failed"))));
    catch_unwind(AssertUnwindSafe(|| {
        if btleplug::platform::init(&env).is_err() {
            return false;
        }
        let Ok(mut run) = RUN.lock() else {
            return false;
        };
        if run.is_some() {
            return false;
        }
        *run = Some(CancellationToken::new());
        true
    }))
    .unwrap_or(false) as jboolean
}

#[no_mangle]
pub extern "system" fn Java_org_tonalli_meshspike_MeshNative_stop(_env: JNIEnv, _class: JClass) {
    if let Ok(run) = RUN.lock() {
        if let Some(stop) = run.as_ref() {
            stop.cancel();
        }
    }
}

#[no_mangle]
pub extern "system" fn Java_org_tonalli_meshspike_MeshNative_run(
    env: JNIEnv,
    _class: JClass,
    config: JString,
    handoff: jboolean,
) -> jstring {
    let result = catch_unwind(AssertUnwindSafe(|| -> Result<String, MeshError> {
        let stop = RUN
            .lock()
            .map_err(|_| MeshError::Connection)?
            .as_ref()
            .cloned()
            .ok_or(MeshError::Cancelled)?;
        let _guard = RunGuard;
        let source: String = env
            .get_string(config)
            .map_err(|_| MeshError::Configuration)?
            .into();
        let config = Config::parse(&source)?;
        if !config.enabled {
            return Ok("disabled; no connection opened".into());
        }
        if config.radio.transport != "ble" {
            return Err(MeshError::UnsupportedTransport);
        }
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|_| MeshError::Connection)?;
        runtime.block_on(async {
            let operation = async {
                let mut radio = gateway::open(&config, handoff != 0, stop).await?;
                let mut report = format!(
                    "ready\nfirmware={}\nnodes={}\nenabled_channels={:?}\n",
                    radio.firmware,
                    radio.list_nodes().await.len(),
                    radio.channels
                );
                let operation = async {
                    if let Some(text) = config.spike.text.as_deref() {
                        radio.send_text(&config, text).await?;
                        report.push_str("send=queued-to-transport; delivery-unconfirmed\n");
                        if config.spike.show_test_text {
                            report.push_str(&format!("sent-test-text={}\n", text.escape_debug()));
                        }
                        let received = radio.receive_text(&config).await?;
                        report.push_str("received=channel-text-from-another-node\n");
                        if config.spike.show_test_text {
                            report.push_str(&format!(
                                "received-test-text={}\n",
                                received.escape_debug()
                            ));
                        }
                    } else {
                        report.push_str("read-only inspection; no text sent\n");
                    }
                    Ok::<_, MeshError>(())
                }
                .await;
                let shutdown = radio.close().await;
                if let Err(error) = operation {
                    report.push_str(&format!("error={error}\n"));
                }
                if let Err(error) = shutdown {
                    report.push_str(&format!("error={error}\n"));
                } else {
                    report.push_str("stopped\n");
                }
                Ok(report)
            }
            .await;
            // The SDK aborts its BLE bridge but does not close Android's cached GATT.
            let released = tokio::time::timeout(
                std::time::Duration::from_secs(5),
                release_ble(&config.radio.selector),
            )
            .await
            .map_err(|_| MeshError::Shutdown)?;
            released?;
            operation.map(|mut report| {
                report.push_str("bluetooth-released\n");
                report
            })
        })
    }));
    let text = match result {
        Ok(Ok(s)) => s,
        Ok(Err(error)) => format!("error={error}"),
        Err(_) => "error=Mesh worker failed".into(),
    };
    env.new_string(text)
        .map(|s| s.into_inner())
        .unwrap_or(std::ptr::null_mut())
}
