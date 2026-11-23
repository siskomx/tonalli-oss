//! An explicitly granted, session-bound text bridge; no radio configuration or automatic replay.
use super::*;
use crate::server::mesh;
use sha2::{Digest, Sha256};

pub(super) struct Bridge {
    pub(super) cancel: CancellationToken,
    view: Arc<Mutex<Value>>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Start {
    room: String,
    channel: u32,
    epoch: u64,
    firmware: String,
    to_broker: bool,
    to_radio: bool,
}
fn nonce(value: &str) -> String {
    format!("{:x}", Sha256::digest(value.as_bytes()))[..32].to_owned()
}
fn messages(value: &Value) -> Result<&Vec<Value>, String> {
    value["messages"]
        .as_array()
        .filter(|v| v.len() <= 100)
        .ok_or_else(|| "Invalid Chat history".into())
}
fn latest(values: &[Value]) -> u64 {
    values
        .iter()
        .filter_map(|m| m["id"].as_u64())
        .max()
        .unwrap_or(0)
}
async fn rpc(
    client: &Client,
    route: &str,
    body: Option<Value>,
    stop: &CancellationToken,
) -> Result<Value, String> {
    let _permit = client
        .slots
        .try_acquire()
        .map_err(|_| "Chat connection is busy")?;
    let future = async {
        let request = client
            .http
            .request(
                if body.is_some() {
                    reqwest::Method::POST
                } else {
                    reqwest::Method::GET
                },
                format!("{}/api/chat{route}", client.origin),
            )
            .header(header::ORIGIN, &client.origin)
            .header(header::COOKIE, &*client.cookie)
            .header("X-Tonalli-Csrf", &client.csrf)
            .header(header::CONTENT_TYPE, "application/json");
        let request = if let Some(body) = body {
            request.body(serde_json::to_vec(&body).map_err(|_| "Invalid bridge request")?)
        } else {
            request
        };
        let response = request
            .send()
            .await
            .map_err(|_| "Chat connection unavailable")?;
        let (status, value) = value(response).await.map_err(|_| "Invalid Chat response")?;
        if !status.is_success() {
            return Err("Chat permission or connection changed");
        }
        Ok(value)
    };
    tokio::select! {biased;_=stop.cancelled()=>Err("Bridge stopped".into()),_=client.cancel.cancelled()=>Err("Chat disconnected".into()),result=future=>result.map_err(str::to_owned)}
}
fn radio(input: &Start) -> Result<Value, String> {
    let value = mesh::snapshot().map_err(|_| "Radio unavailable")?;
    if value["status"] != "connected"
        || value["epoch"] != input.epoch
        || value["firmware"] != input.firmware
        || !value["channels"]
            .as_array()
            .is_some_and(|v| v.iter().any(|c| c["index"] == input.channel))
    {
        return Err("Radio connection or channel changed; bridge stopped".into());
    }
    Ok(value)
}
pub(crate) async fn start(
    State(state): State<Arc<ServerState>>,
    Json(input): Json<Start>,
) -> Response {
    if input.room.is_empty()
        || input.room.len() > 64
        || !input
            .room
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
        || (!input.to_broker && !input.to_radio)
    {
        return StatusCode::BAD_REQUEST.into_response();
    }
    let client = match state.chat.client() {
        Ok(c) => c,
        Err(s) => return s.into_response(),
    };
    if client.user["role"] != "gateway" {
        return StatusCode::FORBIDDEN.into_response();
    }
    let cancel = CancellationToken::new();
    let rooms = match rpc(&client, "/rooms", None, &cancel).await {
        Ok(v) => v,
        Err(_) => return StatusCode::BAD_GATEWAY.into_response(),
    };
    if !rooms["rooms"].as_array().is_some_and(|rooms| {
        rooms
            .iter()
            .any(|r| r["id"] == input.room && r["canBridge"] == true && r["canSend"] == true)
    }) {
        return StatusCode::FORBIDDEN.into_response();
    }
    let initial_radio = match radio(&input) {
        Ok(v) => v,
        Err(_) => return StatusCode::CONFLICT.into_response(),
    };
    let target = format!("/rooms/{}", input.room);
    let initial = match rpc(&client, &target, None, &cancel).await {
        Ok(v) => v,
        Err(_) => return StatusCode::BAD_GATEWAY.into_response(),
    };
    let broker_cursor = match messages(&initial) {
        Ok(v) => latest(v),
        Err(_) => return StatusCode::BAD_GATEWAY.into_response(),
    };
    let radio_cursor = match messages(&initial_radio) {
        Ok(v) => latest(v),
        Err(_) => return StatusCode::BAD_GATEWAY.into_response(),
    };
    let view = Arc::new(Mutex::new(
        json!({"active":true,"room":input.room,"channel":input.channel,"epoch":input.epoch,"to_broker":input.to_broker,"to_radio":input.to_radio,"error":""}),
    ));
    {
        let Ok(mut slot) = state.chat.slot.lock() else {
            return StatusCode::SERVICE_UNAVAILABLE.into_response();
        };
        if !slot
            .client
            .as_ref()
            .is_some_and(|current| Arc::ptr_eq(current, &client))
            || client.cancel.is_cancelled()
        {
            return StatusCode::CONFLICT.into_response();
        }
        if slot
            .bridge
            .as_ref()
            .is_some_and(|b| b.view.lock().map_or(true, |v| v["active"] == true))
        {
            return StatusCode::CONFLICT.into_response();
        }
        slot.bridge = Some(Bridge {
            cancel: cancel.clone(),
            view: Arc::clone(&view),
        });
    }
    std::thread::spawn(move || {
        let result = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(runtime) => runtime.block_on(async {
                let room = input.room.clone();
                let result = run(
                    Arc::clone(&client),
                    input,
                    cancel.clone(),
                    broker_cursor,
                    radio_cursor,
                )
                .await;
                let _ = rpc(
                    &client,
                    &format!("/rooms/{room}/bridge"),
                    Some(json!({"active":false})),
                    &CancellationToken::new(),
                )
                .await;
                result
            }),
            Err(_) => Err("Bridge runtime unavailable".into()),
        };
        cancel.cancel();
        if let Ok(mut value) = view.lock() {
            value["active"] = json!(false);
            value["error"] = json!(result.err().unwrap_or_default());
        }
    });
    Json(json!({"active":true})).into_response()
}
async fn run(
    client: Arc<Client>,
    input: Start,
    cancel: CancellationToken,
    mut broker_cursor: u64,
    mut radio_cursor: u64,
) -> Result<(), String> {
    let target = format!("/rooms/{}", input.room);
    loop {
        tokio::select! {biased;_=cancel.cancelled()=>return Ok(()),_=client.cancel.cancelled()=>return Err("Chat disconnected".into()),_=tokio::time::sleep(Duration::from_secs(2))=>{}}
        let snapshot = radio(&input)?;
        let incoming = messages(&snapshot)?;
        let channel_name = snapshot["channels"]
            .as_array()
            .and_then(|v| v.iter().find(|c| c["index"] == input.channel))
            .and_then(|c| c["name"].as_str())
            .ok_or("Radio channel changed")?;
        rpc(&client,&format!("{target}/bridge"),Some(json!({"active":true,"channel":input.channel,"name":channel_name,"to_radio":input.to_radio,"to_broker":input.to_broker})),&cancel).await?;
        if incoming.len() == 100
            && incoming
                .first()
                .and_then(|v| v["id"].as_u64())
                .is_some_and(|id| id > radio_cursor)
        {
            return Err("Radio history advanced too far; restart the bridge explicitly".into());
        }
        if input.to_broker {
            for message in incoming.iter().filter(|m| {
                m["id"].as_u64().is_some_and(|id| id > radio_cursor)
                    && m["channel"] == input.channel
                    && m["origin"] == "radio"
            }) {
                let packet = message["packet"]
                    .as_u64()
                    .filter(|p| *p != 0)
                    .ok_or("Radio packet has no replay identity")?;
                let identity = nonce(&format!(
                    "radio:{}:{}:{}:{packet}",
                    input.room, input.channel, message["node"]
                ));
                rpc(&client,&target,Some(json!({"text":message["text"],"nonce":identity,"origin":"radio","node":message["node"],"channel":input.channel})),&cancel).await?;
            }
        }
        radio_cursor = radio_cursor.max(latest(incoming));
        let history = rpc(&client, &target, None, &cancel).await?;
        let outgoing = messages(&history)?;
        if outgoing.len() == 100
            && outgoing
                .first()
                .and_then(|v| v["id"].as_u64())
                .is_some_and(|id| id > broker_cursor)
        {
            return Err("Chat history advanced too far; restart the bridge explicitly".into());
        }
        let pending: Vec<_> = outgoing
            .iter()
            .filter(|m| {
                m["id"].as_u64().is_some_and(|id| id > broker_cursor) && m["origin"] == "broker"
            })
            .collect();
        if input.to_radio && pending.len() > 8 {
            return Err("Too many pending radio messages; bridge stopped without replay".into());
        }
        if input.to_radio {
            for message in pending {
                radio(&input)?;
                rpc(&client,&format!("{target}/bridge"),Some(json!({"active":true,"channel":input.channel,"name":channel_name,"to_radio":input.to_radio,"to_broker":input.to_broker})),&cancel).await?;
                let name = message["actor"]["name"]
                    .as_str()
                    .filter(|s| s.len() <= 80)
                    .ok_or("Invalid Chat sender")?;
                let text = message["text"].as_str().ok_or("Invalid Chat text")?;
                let text = format!("[Chat: {name}] {text}");
                if text.len() > 200 {
                    return Err("Message exceeds radio length; bridge stopped".into());
                }
                let id = nonce(&format!(
                    "{}:{}:{}:{}",
                    client.origin, input.room, message["id"], message["time"]
                ));
                let send = mesh::send_owned(
                    mesh::Text {
                        epoch: input.epoch,
                        channel: input.channel,
                        firmware: input.firmware.clone(),
                        text,
                        nonce: id,
                    },
                    cancel.clone(),
                );
                let response = tokio::select! {biased;_=cancel.cancelled()=>return Ok(()),_=client.cancel.cancelled()=>return Err("Chat disconnected".into()),result=send=>result};
                if !response.status().is_success() {
                    return Err("Radio delivery unknown; bridge stopped without replay".into());
                }
                tokio::select! {biased;_=cancel.cancelled()=>return Ok(()),_=client.cancel.cancelled()=>return Err("Chat disconnected".into()),_=tokio::time::sleep(Duration::from_secs(5))=>{}}
            }
        }
        broker_cursor = broker_cursor.max(latest(outgoing));
    }
}
pub(crate) async fn status(State(state): State<Arc<ServerState>>) -> Response {
    let Ok(slot) = state.chat.slot.lock() else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    match slot.bridge.as_ref() {
        Some(bridge) => match bridge.view.lock() {
            Ok(view) => Json(view.clone()).into_response(),
            Err(_) => StatusCode::SERVICE_UNAVAILABLE.into_response(),
        },
        None => Json(json!({"active":false})).into_response(),
    }
}
pub(crate) async fn stop(State(state): State<Arc<ServerState>>) -> Json<Value> {
    state.chat.stop_bridge();
    Json(json!({"stopping":true}))
}
