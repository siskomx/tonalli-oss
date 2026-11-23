//! One native radio owner. The browser supplies commands, never a transport or node identity.
use super::ServerState;
use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::{Arc, Mutex, OnceLock};
use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;
use tonalli_mesh_gateway::{config::Config, gateway};

#[derive(Clone)]
pub(super) struct Owner {
    state: Arc<Mutex<Slot>>,
    scan: Arc<tokio::sync::Mutex<()>>,
}
static OWNER: OnceLock<Owner> = OnceLock::new();
#[cfg(target_os = "android")]
static ANDROID_READY: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
impl Default for Owner {
    fn default() -> Self {
        OWNER
            .get_or_init(|| Self {
                state: Arc::new(Mutex::new(Slot::default())),
                scan: Arc::new(tokio::sync::Mutex::new(())),
            })
            .clone()
    }
}
#[cfg_attr(feature = "ffi", uniffi::export)]
pub fn mesh_radio_stop() {
    #[cfg(target_os = "android")]
    ANDROID_READY.store(false, std::sync::atomic::Ordering::SeqCst);
    #[cfg(target_os = "android")]
    super::connections::Connection::default().stop_bridge();
    Owner::default().stop();
}
#[cfg_attr(feature = "ffi", uniffi::export)]
pub fn mesh_radio_active() -> bool {
    Owner::default()
        .state
        .lock()
        .map_or(true, |s| s.stop.is_some())
}
#[cfg(all(target_os = "android", feature = "mesh-android"))]
#[no_mangle]
pub extern "system" fn Java_org_tonalli_spike_mesh_MeshNative_initialize(
    env: jni::JNIEnv,
    _class: jni::objects::JClass,
) -> jni::sys::jboolean {
    let ready = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        tonalli_mesh_gateway::initialize_android(&env)
    }))
    .unwrap_or(false);
    ANDROID_READY.store(ready, std::sync::atomic::Ordering::SeqCst);
    ready as jni::sys::jboolean
}
#[derive(Default)]
struct Slot {
    epoch: u64,
    status: String,
    firmware: String,
    channels: Vec<gateway::ChannelInfo>,
    nodes: Vec<gateway::NodeInfo>,
    messages: Vec<Value>,
    error: String,
    stop: Option<CancellationToken>,
    commands: Option<mpsc::Sender<Send>>,
}
struct Send {
    channel: u32,
    firmware: String,
    text: String,
    nonce: String,
    admission: CancellationToken,
    reply: oneshot::Sender<Result<(), ()>>,
}
fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(0))
}
fn history(
    directory: &std::path::Path,
) -> Result<tonalli_chat_store::Store, tonalli_chat_store::Error> {
    super::chat_storage::open(directory)
}
fn history_room(config: &Config, channel: u32) -> String {
    use sha2::{Digest, Sha256};
    format!(
        "{:x}",
        Sha256::digest(format!(
            "{}:{}:{channel}",
            config.radio.transport, config.radio.selector
        ))
    )
}
impl Owner {
    #[cfg(not(target_os = "android"))]
    pub(super) fn wait_stopped(&self) -> Result<(), crate::pack::PackError> {
        let until = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while self.state.lock().map_or(true, |s| s.stop.is_some()) {
            if std::time::Instant::now() >= until {
                return Err(crate::pack::PackError::Io(
                    "Radio cleanup did not finish".into(),
                ));
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        Ok(())
    }
    pub(super) fn stop(&self) {
        if let Ok(mut s) = self.state.lock() {
            if let Some(stop) = s.stop.as_ref() {
                stop.cancel();
            }
            s.status = if s.stop.is_some() {
                "stopping"
            } else {
                "disconnected"
            }
            .into();
            s.commands = None;
        }
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Connect {
    transport: String,
    selector: String,
    handoff: bool,
}
pub(super) async fn connect(
    State(state): State<Arc<ServerState>>,
    Json(input): Json<Connect>,
) -> Response {
    #[cfg(target_os = "android")]
    if !ANDROID_READY.load(std::sync::atomic::Ordering::SeqCst) {
        return (
            StatusCode::FORBIDDEN,
            Json(json!({"error":"Enable radio on this phone before connecting."})),
        )
            .into_response();
    }
    let Ok(_scan) = state.mesh.scan.try_lock() else {
        return StatusCode::CONFLICT.into_response();
    };
    if !input.handoff
        || !matches!(input.transport.as_str(), "serial" | "ble")
        || input.selector.is_empty()
        || input.selector.len() > 4096
        || input.selector.chars().any(char::is_control)
    {
        return StatusCode::BAD_REQUEST.into_response();
    }
    let mut config = Config::default();
    config.enabled = true;
    config.radio.transport = input.transport;
    config.radio.selector = input.selector;
    config.radio.lock_directory = state.data_dir.join("chat-radio-locks");
    let (commands, mut receiver) = mpsc::channel::<Send>(8);
    let stop = CancellationToken::new();
    let owner = Arc::clone(&state.mesh.state);
    let directory = state.data_dir.clone();
    let epoch = {
        let Ok(mut s) = owner.lock() else {
            return StatusCode::SERVICE_UNAVAILABLE.into_response();
        };
        if s.stop.is_some() {
            return StatusCode::CONFLICT.into_response();
        }
        let epoch = s.epoch.wrapping_add(1);
        *s = Slot {
            epoch,
            status: "connecting".into(),
            stop: Some(stop.clone()),
            commands: Some(commands),
            ..Slot::default()
        };
        epoch
    };
    let failed_owner = Arc::clone(&owner);
    let spawned=std::thread::Builder::new().name("tonalli-radio".into()).spawn(move || {
        #[cfg(target_os = "android")]
        let _android_thread = match tonalli_mesh_gateway::attach_android_thread() {
            Ok(guard) => guard,
            Err(_) => {
                if let Ok(mut s) = owner.lock() {
                    s.status = "disconnected".into();
                    s.error = "Enable radio on this phone before connecting".into();
                    s.stop = None;
                    s.commands = None;
                }
                return;
            }
        };
        let runtime = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(r) => r,
            Err(_) => {
                if let Ok(mut s) = owner.lock() {
                    s.status = "disconnected".into();
                    s.error = "Radio runtime unavailable".into();
                    s.stop = None;
                    s.commands = None;
                }
                return;
            }
        };
        runtime.block_on(async move {
        let mut journal=match history(&directory){Ok(s)=>s,Err(_)=>{if let Ok(mut s)=owner.lock(){s.status="disconnected".into();s.error="Encrypted radio history unavailable".into();s.stop=None;s.commands=None;}return;}};
        let result=gateway::open(&config,true,stop.clone()).await;
        let mut radio=match result {Ok(r)=>r,Err(e)=>{if let Ok(mut s)=owner.lock(){if s.epoch==epoch{s.status="disconnected".into();s.error=e.to_string();s.stop=None;s.commands=None;}}return;}};
        let previous=(||->Result<Vec<Value>,tonalli_chat_store::Error>{
            let channels:Vec<_>=radio.channel_info.iter().map(|channel|json!({"channel":channel,"room":history_room(&config,channel.index)})).collect();
            journal.save_radio_metadata(&json!({"firmware":radio.firmware,"channels":channels}))?;
            let mut messages=Vec::new();for channel in &radio.channel_info{messages.extend(journal.list(&history_room(&config,channel.index))?);}messages.sort_by_key(|m|m["id"].as_i64().unwrap_or(0));if messages.len()>100{messages.drain(..messages.len()-100);}Ok(messages)
        })();
        let previous=match previous{Ok(v)=>v,Err(_)=>{let _=radio.close().await;if let Ok(mut s)=owner.lock(){s.status="disconnected".into();s.error="Encrypted radio history unavailable".into();s.stop=None;s.commands=None;}return;}};
        if let Ok(mut s)=owner.lock(){if s.epoch==epoch{s.status="connected".into();s.firmware=radio.firmware.clone();s.channels=radio.channel_info.clone();s.nodes=radio.node_directory();s.messages=previous;}}
        loop {
            tokio::select!{
                _=stop.cancelled()=>break,
                command=receiver.recv()=>{
                    let Some(command)=command else{break;};
                    if command.admission.is_cancelled(){let _=command.reply.send(Err(()));continue;}
                    let staged=journal.append(&history_room(&config,command.channel),"local",&command.nonce,&json!({"channel":command.channel,"text":command.text,"origin":"local","status":"delivery_unknown","time":now_ms()}));
                    let (mut message,inserted)=match staged{Ok(v)=>v,Err(_)=>{let _=command.reply.send(Err(()));break;}};
                    if !inserted {let _=command.reply.send(if message["status"]=="radio_submitted"{Ok(())}else{Err(())});continue;}
                    config.radio.channel_index=Some(command.channel);config.radio.accepted_firmware=vec![command.firmware];
                    let mut success=tokio::select!{biased;_=command.admission.cancelled()=>false,v=tokio::time::timeout(std::time::Duration::from_secs(5),radio.send_text(&config,&command.text))=>matches!(v,Ok(Ok(())))};
                    if success {if let Some(id)=message["id"].as_i64(){success=journal.status(id,"radio_submitted").is_ok();}else{success=false;}}
                    if success{message["status"]=json!("radio_submitted");}
                    if let Ok(mut s)=owner.lock(){if s.epoch==epoch {s.messages.push(message);if s.messages.len()>100{s.messages.remove(0);}}}
                    let _=command.reply.send(if success{Ok(())}else{Err(())});
                    if !success{break;}
                },
                event=radio.receive_event()=>{
                    match event {Ok(gateway::RadioEvent::NodesChanged)=>{if let Ok(mut s)=owner.lock(){if s.epoch==epoch{s.nodes=radio.node_directory();}}},Ok(gateway::RadioEvent::Text(event))=>{
                        let nonce=if event.id!=0{format!("{}:{}",event.from,event.id)}else{let mut bytes=[0u8;16];if getrandom::fill(&mut bytes).is_err(){break;}format!("{:x}",u128::from_le_bytes(bytes))};
                        let saved=journal.append(&history_room(&config,event.channel),"radio",&nonce,&json!({"channel":event.channel,"text":event.text,"node":format!("!{:08x}",event.from),"packet":event.id,"origin":"radio","status":"received","time":now_ms()}));
                        let (message,inserted)=match saved{Ok(v)=>v,Err(_)=>break,};
                        if inserted{if let Ok(mut s)=owner.lock(){if s.epoch==epoch{s.messages.push(message);if s.messages.len()>100{s.messages.remove(0);}}}}
                    },Err(e)=>{if let Ok(mut s)=owner.lock(){if s.epoch==epoch{s.error=e.to_string();}}break;}}
                }
            }
        }
        let closed=radio.close().await;
        if let Ok(mut s)=owner.lock(){if s.epoch==epoch{s.status="disconnected".into();s.stop=None;s.commands=None;if closed.is_err(){s.error="Radio release failed".into();}}}
        });
    });
    if spawned.is_err() {
        if let Ok(mut s) = failed_owner.lock() {
            if s.epoch == epoch {
                if let Some(stop) = s.stop.take() {
                    stop.cancel();
                }
                s.commands = None;
                s.status = "disconnected".into();
                s.error = "Radio worker unavailable".into();
            }
        }
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    }
    Json(json!({"connecting":true,"epoch":epoch})).into_response()
}
pub(super) async fn status(State(state): State<Arc<ServerState>>) -> Response {
    let owner = Owner::default();
    let restore = owner
        .state
        .lock()
        .ok()
        .filter(|s| s.stop.is_none() && s.channels.is_empty())
        .map(|s| s.epoch);
    if let Some(epoch) = restore {
        let directory = state.data_dir.clone();
        let restored =
            tokio::task::spawn_blocking(move || -> Result<_, tonalli_chat_store::Error> {
                let journal = super::chat_storage::open(&directory)?;
                let Some(metadata) = journal.radio_metadata()? else {
                    return Ok(None);
                };
                let firmware = metadata["firmware"]
                    .as_str()
                    .filter(|s| s.len() <= 128 && s.is_ascii() && !s.chars().any(char::is_control))
                    .ok_or("Invalid radio history")?
                    .to_owned();
                let entries = metadata["channels"]
                    .as_array()
                    .filter(|v| v.len() <= 8)
                    .ok_or("Invalid radio history")?;
                let mut channels = Vec::new();
                let mut messages = Vec::new();
                let mut seen = std::collections::BTreeSet::new();
                for entry in entries {
                    let channel: gateway::ChannelInfo =
                        serde_json::from_value(entry["channel"].clone())?;
                    if channel.index > 7
                        || !seen.insert(channel.index)
                        || channel.name.len() > 256
                        || channel.name.chars().any(char::is_control)
                    {
                        return Err("Invalid channel history".into());
                    }
                    let room = entry["room"]
                        .as_str()
                        .filter(|s| s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit()))
                        .ok_or("Invalid radio history")?;
                    messages.extend(journal.list(room)?);
                    channels.push(channel);
                }
                messages.sort_by_key(|m| m["id"].as_i64().unwrap_or(0));
                if messages.len() > 100 {
                    messages.drain(..messages.len() - 100);
                }
                Ok(Some((firmware, channels, messages)))
            })
            .await;
        if let Ok(mut s) = owner.state.lock() {
            if s.epoch == epoch && s.stop.is_none() && s.channels.is_empty() {
                match restored {
                    Ok(Ok(Some((firmware, channels, messages)))) => {
                        s.firmware = firmware;
                        s.channels = channels;
                        s.messages = messages;
                        s.status = "disconnected".into();
                    }
                    Ok(Ok(None)) => {}
                    _ => s.error = "Encrypted radio history unavailable".into(),
                }
            }
        }
    }
    match snapshot() {
        Ok(v) => Json(v).into_response(),
        Err(s) => s.into_response(),
    }
}
pub(super) fn snapshot() -> Result<Value, StatusCode> {
    let owner = Owner::default();
    let s = owner
        .state
        .lock()
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    Ok(
        json!({"epoch":s.epoch,"status":s.status,"firmware":s.firmware,"channels":s.channels,"nodes":s.nodes,"messages":s.messages,"error":s.error}),
    )
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Text {
    pub epoch: u64,
    pub channel: u32,
    pub firmware: String,
    pub text: String,
    pub nonce: String,
}
pub(super) async fn send(
    State(state): State<Arc<ServerState>>,
    Json(input): Json<Text>,
) -> Response {
    let _ = state;
    send_owned(input, CancellationToken::new()).await
}
pub(super) async fn send_owned(input: Text, admission: CancellationToken) -> Response {
    let owner = Owner::default();
    if input.text.is_empty()
        || input.text.len() > 200
        || input.text.chars().any(|c| c.is_control() && c != '\n')
        || input.nonce.len() != 32
        || !input.nonce.bytes().all(|b| b.is_ascii_hexdigit())
    {
        return StatusCode::BAD_REQUEST.into_response();
    }
    let (reply, result) = oneshot::channel();
    {
        let Ok(s) = owner.state.lock() else {
            return StatusCode::SERVICE_UNAVAILABLE.into_response();
        };
        if s.epoch != input.epoch
            || s.status != "connected"
            || s.firmware != input.firmware
            || !s.channels.iter().any(|c| c.index == input.channel)
        {
            return StatusCode::CONFLICT.into_response();
        }
        let Some(commands) = s.commands.as_ref() else {
            return StatusCode::CONFLICT.into_response();
        };
        if commands
            .try_send(Send {
                channel: input.channel,
                firmware: input.firmware,
                text: input.text,
                nonce: input.nonce,
                admission,
                reply,
            })
            .is_err()
        {
            return StatusCode::TOO_MANY_REQUESTS.into_response();
        }
    }
    match result.await {
        Ok(Ok(())) => Json(json!({"status":"radio_submitted"})).into_response(),
        _ => (
            StatusCode::BAD_GATEWAY,
            Json(json!({"error":"Delivery unknown. Check the conversation before sending again."})),
        )
            .into_response(),
    }
}
pub(super) async fn stop(State(state): State<Arc<ServerState>>) -> Json<Value> {
    state.chat.stop_bridge();
    state.mesh.stop();
    Json(json!({"stopping":true}))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Discover {
    transport: String,
}
pub(super) async fn devices(
    State(state): State<Arc<ServerState>>,
    Json(input): Json<Discover>,
) -> Response {
    #[cfg(target_os = "android")]
    if !ANDROID_READY.load(std::sync::atomic::Ordering::SeqCst) {
        return (
            StatusCode::FORBIDDEN,
            Json(json!({"error":"Enable radio on this phone before finding radios."})),
        )
            .into_response();
    }
    #[cfg(not(target_os = "android"))]
    let Ok(_scan) = state.mesh.scan.try_lock() else {
        return StatusCode::CONFLICT.into_response();
    };
    #[cfg(target_os = "android")]
    let Ok(scan) = state.mesh.scan.clone().try_lock_owned() else {
        return StatusCode::CONFLICT.into_response();
    };
    {
        let Ok(s) = state.mesh.state.lock() else {
            return StatusCode::SERVICE_UNAVAILABLE.into_response();
        };
        if s.stop.is_some() {
            return StatusCode::CONFLICT.into_response();
        }
    }
    #[cfg(target_os = "android")]
    let result = tokio::task::spawn_blocking(move || {
        let _scan = scan;
        let _attached = tonalli_mesh_gateway::attach_android_thread()?;
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|_| tonalli_mesh_gateway::MeshError::Connection)?;
        runtime.block_on(gateway::devices(&input.transport))
    })
    .await
    .map_err(|_| tonalli_mesh_gateway::MeshError::Connection)
    .and_then(|v| v);
    #[cfg(not(target_os = "android"))]
    let result = gateway::devices(&input.transport).await;
    match result {Ok(devices)=>Json(json!({"devices":devices})).into_response(),Err(_)=>(StatusCode::SERVICE_UNAVAILABLE,Json(json!({"error":"No radio available. Check Bluetooth permission or the USB connection."}))).into_response()}
}
