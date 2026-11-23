use crate::{config::Config, MeshError};
use fs2::FileExt;
use meshtastic::{
    api::{ConnectedStreamApi, StreamApi},
    packet::{PacketDestination, PacketReceiver, PacketRouter},
    protobufs::{self, from_radio::PayloadVariant},
    types::{MeshChannel, NodeId},
};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{File, OpenOptions},
    future::Future,
    path::Path,
    time::Duration,
};
use tokio_util::sync::CancellationToken;

pub trait RadioTransport {
    fn receive(&mut self) -> impl Future<Output = Result<protobufs::FromRadio, MeshError>> + Send;
    fn send_text(
        &mut self,
        node: u32,
        channel: u32,
        text: &str,
    ) -> impl Future<Output = Result<(), MeshError>> + Send;
    fn close(&mut self) -> impl Future<Output = Result<(), MeshError>> + Send;
}

pub struct LiveTransport {
    api: Option<ConnectedStreamApi>,
    receiver: PacketReceiver,
    _lease: File,
    #[cfg(feature = "ble")]
    ble_selector: Option<String>,
}
struct IgnoreLocalEcho(NodeId);
impl PacketRouter<(), std::convert::Infallible> for IgnoreLocalEcho {
    fn handle_packet_from_radio(
        &mut self,
        _packet: protobufs::FromRadio,
    ) -> Result<(), std::convert::Infallible> {
        Ok(())
    }
    fn handle_mesh_packet(
        &mut self,
        _packet: protobufs::MeshPacket,
    ) -> Result<(), std::convert::Infallible> {
        Ok(())
    }
    fn source_node_id(&self) -> NodeId {
        self.0
    }
}
impl RadioTransport for LiveTransport {
    async fn receive(&mut self) -> Result<protobufs::FromRadio, MeshError> {
        self.receiver.recv().await.ok_or(MeshError::Disconnected)
    }
    async fn send_text(&mut self, node: u32, channel: u32, text: &str) -> Result<(), MeshError> {
        let api = self.api.as_mut().ok_or(MeshError::Disconnected)?;
        let channel = MeshChannel::from(channel);
        api.send_text(
            &mut IgnoreLocalEcho(node.into()),
            text.into(),
            PacketDestination::Broadcast,
            false,
            channel,
        )
        .await
        .map_err(|_| MeshError::Connection)
    }
    async fn close(&mut self) -> Result<(), MeshError> {
        let mut result = Ok(());
        if let Some(api) = self.api.take() {
            result = tokio::time::timeout(Duration::from_secs(3), api.disconnect())
                .await
                .map_err(|_| MeshError::Shutdown)
                .and_then(|value| value.map(|_| ()).map_err(|_| MeshError::Shutdown));
        }
        #[cfg(feature = "ble")]
        if let Some(selector) = self.ble_selector.take() {
            let released = tokio::time::timeout(Duration::from_secs(5), release_ble(&selector))
                .await
                .map_err(|_| MeshError::Shutdown)
                .and_then(|r| r);
            if released.is_err() {
                result = released;
            }
        }
        result
    }
}
#[cfg(feature = "ble")]
async fn release_ble(selector: &str) -> Result<(), MeshError> {
    use btleplug::api::{Central, Manager as _, Peripheral as _};
    let manager = btleplug::platform::Manager::new()
        .await
        .map_err(|_| MeshError::Shutdown)?;
    for adapter in manager.adapters().await.map_err(|_| MeshError::Shutdown)? {
        let _ = adapter.stop_scan().await;
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConnectionState {
    Ready,
    Stopped,
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChannelInfo {
    pub index: u32,
    pub name: String,
    pub uplink_enabled: bool,
    pub downlink_enabled: bool,
}
pub struct RadioText {
    pub from: u32,
    pub channel: u32,
    pub id: u32,
    pub text: String,
}
#[derive(Clone, PartialEq, serde::Serialize)]
pub struct NodeInfo {
    pub id: String,
    pub long_name: String,
    pub short_name: String,
}
pub enum RadioEvent {
    Text(RadioText),
    NodesChanged,
}
fn update_node(
    nodes: &mut BTreeMap<u32, NodeInfo>,
    info: protobufs::NodeInfo,
    limit: usize,
) -> Result<bool, MeshError> {
    if info.num == 0 || (!nodes.contains_key(&info.num) && nodes.len() >= limit) {
        return Err(MeshError::Capabilities);
    }
    let label = |value: &str, maximum| {
        value
            .chars()
            .filter(|c| !c.is_control())
            .take(maximum)
            .collect()
    };
    let node = NodeInfo {
        id: format!("!{:08x}", info.num),
        long_name: label(info.user.as_ref().map_or("", |u| &u.long_name), 64),
        short_name: label(info.user.as_ref().map_or("", |u| &u.short_name), 16),
    };
    if nodes.get(&info.num) == Some(&node) {
        return Ok(false);
    }
    nodes.insert(info.num, node);
    Ok(true)
}
#[derive(serde::Serialize)]
pub struct Device {
    pub selector: String,
    pub name: String,
}
pub async fn devices(transport: &str) -> Result<Vec<Device>, MeshError> {
    match transport {
        "serial" => Ok(meshtastic::utils::stream::available_serial_ports()
            .map_err(|_| MeshError::Connection)?
            .into_iter()
            .take(32)
            .map(|selector| Device {
                name: selector.clone(),
                selector,
            })
            .collect()),
        #[cfg(feature = "ble")]
        "ble" => Ok(
            meshtastic::utils::stream::available_ble_devices(Duration::from_secs(3))
                .await
                .map_err(|_| MeshError::Connection)?
                .into_iter()
                .take(32)
                .map(|device| Device {
                    selector: device.mac_address.to_string(),
                    name: device
                        .name
                        .unwrap_or_else(|| "Meshtastic".into())
                        .chars()
                        .filter(|c| !c.is_control())
                        .take(64)
                        .collect(),
                })
                .collect(),
        ),
        _ => Err(MeshError::UnsupportedTransport),
    }
}

pub struct Gateway<T: RadioTransport> {
    transport: T,
    pub firmware: String,
    pub channels: BTreeSet<u32>,
    pub channel_info: Vec<ChannelInfo>,
    nodes: BTreeMap<u32, NodeInfo>,
    node_limit: usize,
    own_node: u32,
    state: ConnectionState,
    stop: CancellationToken,
}

fn lease(config: &Config) -> Result<File, MeshError> {
    let directory = &config.radio.lock_directory;
    if !directory.is_absolute() {
        return Err(MeshError::Configuration);
    }
    std::fs::create_dir_all(directory).map_err(|_| MeshError::Configuration)?;
    let directory = directory
        .canonicalize()
        .map_err(|_| MeshError::Configuration)?;
    if directory.ancestors().any(|p| p.join(".git").exists()) {
        return Err(MeshError::Configuration);
    }
    let selector = if config.radio.transport == "ble" {
        config.radio.selector.replace(':', "").to_ascii_lowercase()
    } else {
        config.radio.selector.clone()
    };
    let identity = format!("{}:{selector}", config.radio.transport);
    let path = directory.join(format!("{:x}.lock", Sha256::digest(identity.as_bytes())));
    if path
        .symlink_metadata()
        .is_ok_and(|m| m.file_type().is_symlink() || !m.is_file())
    {
        return Err(MeshError::Configuration);
    }
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&path)
        .map_err(|_| MeshError::Configuration)?;
    file.try_lock_exclusive().map_err(|_| MeshError::Busy)?;
    Ok(file)
}

pub async fn open(
    config: &Config,
    handoff_confirmed: bool,
    stop: CancellationToken,
) -> Result<Gateway<LiveTransport>, MeshError> {
    if !config.enabled {
        return Err(MeshError::Configuration);
    }
    if !handoff_confirmed {
        return Err(MeshError::HandoffRequired);
    }
    if stop.is_cancelled() {
        return Err(MeshError::Cancelled);
    }
    let owner = lease(config)?;
    let nonce = meshtastic::utils::generate_rand_id::<u32>();
    let connection = async {
        let (receiver, api) = match config.radio.transport.as_str() {
            "serial" => {
                let stream = meshtastic::utils::stream::build_serial_stream(
                    config.radio.selector.clone(),
                    Some(config.radio.baud),
                    Some(config.radio.dtr),
                    Some(config.radio.rts),
                )
                .map_err(|_| MeshError::Connection)?;
                #[cfg(unix)]
                let stream = {
                    let mut stream = stream;
                    stream
                        .stream
                        .set_exclusive(true)
                        .map_err(|_| MeshError::Busy)?;
                    stream
                };
                let (receiver, api) = StreamApi::new().connect(stream).await;
                (
                    receiver,
                    api.configure(nonce)
                        .await
                        .map_err(|_| MeshError::Connection)?,
                )
            }
            "ble" => {
                #[cfg(feature = "ble")]
                {
                    let selected =
                        meshtastic::utils::stream::BleId::from_mac_address(&config.radio.selector)
                            .map_err(|_| MeshError::Configuration)?;
                    let stream = meshtastic::utils::stream::build_ble_stream(
                        &selected,
                        // The SDK waits the entire scan duration before connecting.
                        Duration::from_millis(
                            (config.radio.connect_timeout_seconds * 1000 / 3).min(5000),
                        ),
                    )
                    .await
                    .map_err(|_| MeshError::Connection)?;
                    let (receiver, api) = StreamApi::new().connect(stream).await;
                    (
                        receiver,
                        api.configure(nonce)
                            .await
                            .map_err(|_| MeshError::Connection)?,
                    )
                }
                #[cfg(not(feature = "ble"))]
                {
                    return Err(MeshError::UnsupportedTransport);
                }
            }
            _ => return Err(MeshError::UnsupportedTransport),
        };
        Ok::<_, MeshError>((receiver, api))
    };
    let connected = tokio::select! {
        biased;
        _=stop.cancelled()=>Err(MeshError::Cancelled),
        value=tokio::time::timeout(Duration::from_secs(config.radio.connect_timeout_seconds),connection)=>value.map_err(|_|MeshError::Connection).and_then(|r|r),
    };
    let (receiver, api) = match connected {
        Ok(value) => value,
        Err(error) => {
            #[cfg(feature = "ble")]
            if config.radio.transport == "ble" {
                let _ = tokio::time::timeout(
                    Duration::from_secs(5),
                    release_ble(&config.radio.selector),
                )
                .await;
            }
            return Err(error);
        }
    };
    let transport = LiveTransport {
        api: Some(api),
        receiver,
        _lease: owner,
        #[cfg(feature = "ble")]
        ble_selector: (config.radio.transport == "ble").then(|| config.radio.selector.clone()),
    };
    Gateway::configure(transport, nonce, config, stop).await
}

impl<T: RadioTransport + Send> Gateway<T> {
    async fn configure(
        mut transport: T,
        nonce: u32,
        config: &Config,
        stop: CancellationToken,
    ) -> Result<Self, MeshError> {
        let mut firmware = None;
        let mut own_node = None;
        let mut nodes = BTreeMap::new();
        let mut channels = BTreeSet::new();
        let mut channel_info = Vec::new();
        let mut complete = false;
        let read = async {
            for _ in 0..config.radio.max_config_events {
                match transport.receive().await?.payload_variant {
                    Some(PayloadVariant::MyInfo(info)) => {
                        own_node = Some(info.my_node_num);
                    }
                    Some(PayloadVariant::Metadata(info)) => {
                        if info.firmware_version.is_empty()
                            || info.firmware_version.len() > 128
                            || !info.firmware_version.is_ascii()
                            || info.firmware_version.chars().any(char::is_control)
                        {
                            return Err(MeshError::Capabilities);
                        }
                        firmware = Some(info.firmware_version);
                    }
                    Some(PayloadVariant::NodeInfo(info)) => {
                        update_node(&mut nodes, info, config.radio.max_nodes)?;
                    }
                    Some(PayloadVariant::Channel(mut channel)) => {
                        if let Some(settings) = channel.settings.as_mut() {
                            settings.psk.fill(0);
                        }
                        if matches!(
                            protobufs::channel::Role::try_from(channel.role),
                            Ok(protobufs::channel::Role::Primary
                                | protobufs::channel::Role::Secondary)
                        ) {
                            let index = u32::try_from(channel.index)
                                .map_err(|_| MeshError::Capabilities)?;
                            if index > 7 || !channels.insert(index) {
                                return Err(MeshError::Capabilities);
                            }
                            let settings =
                                channel.settings.as_ref().ok_or(MeshError::Capabilities)?;
                            let name: String = settings
                                .name
                                .chars()
                                .filter(|c| !c.is_control())
                                .take(64)
                                .collect();
                            channel_info.push(ChannelInfo {
                                index,
                                name: if name.is_empty() {
                                    format!("Channel {}", index + 1)
                                } else {
                                    name
                                },
                                uplink_enabled: settings.uplink_enabled,
                                downlink_enabled: settings.downlink_enabled,
                            });
                        }
                    }
                    Some(PayloadVariant::ConfigCompleteId(id)) => {
                        complete |= id == nonce;
                    }
                    _ => {}
                }
                if complete && firmware.is_some() && own_node.is_some() {
                    return Ok(());
                }
            }
            Err(MeshError::Capabilities)
        };
        let result = tokio::select! {
            biased;
            _=stop.cancelled()=>Err(MeshError::Cancelled),
            value=tokio::time::timeout(Duration::from_secs(config.radio.config_timeout_seconds),read)=>value.map_err(|_|MeshError::ConfigurationTimeout).and_then(|r|r),
        };
        if let Err(error) = result {
            let _ = transport.close().await;
            return Err(error);
        }
        Ok(Self {
            transport,
            firmware: firmware.ok_or(MeshError::Capabilities)?,
            channels,
            channel_info,
            nodes,
            node_limit: config.radio.max_nodes,
            own_node: own_node.ok_or(MeshError::Capabilities)?,
            state: ConnectionState::Ready,
            stop,
        })
    }
    pub fn state(&self) -> ConnectionState {
        if self.stop.is_cancelled() {
            ConnectionState::Stopped
        } else {
            self.state
        }
    }
    pub async fn list_nodes(&self) -> Vec<u32> {
        self.nodes.keys().copied().collect()
    }
    pub fn node_directory(&self) -> Vec<NodeInfo> {
        self.nodes.values().cloned().collect()
    }
    pub async fn send_text(&mut self, config: &Config, text: &str) -> Result<(), MeshError> {
        let channel = config
            .radio
            .channel_index
            .ok_or(MeshError::SendNotAuthorized)?;
        if self.state != ConnectionState::Ready
            || !config.radio.accepted_firmware.contains(&self.firmware)
            || !self.channels.contains(&channel)
            || text.is_empty()
            || text.len() > 200
            || text.contains('\0')
        {
            return Err(MeshError::SendNotAuthorized);
        }
        tokio::select! {biased;_=self.stop.cancelled()=>Err(MeshError::Cancelled),r=self.transport.send_text(self.own_node,channel,text)=>r}
    }
    pub async fn receive_text(&mut self, config: &Config) -> Result<String, MeshError> {
        if self.state != ConnectionState::Ready {
            return Err(MeshError::Disconnected);
        }
        let channel = config
            .radio
            .channel_index
            .ok_or(MeshError::SendNotAuthorized)?;
        let receive = async {
            loop {
                if let Some(PayloadVariant::Packet(packet)) =
                    self.transport.receive().await?.payload_variant
                {
                    if packet.from == self.own_node || packet.channel != channel {
                        continue;
                    }
                    if let Some(protobufs::mesh_packet::PayloadVariant::Decoded(data)) =
                        packet.payload_variant
                    {
                        if data.portnum == protobufs::PortNum::TextMessageApp as i32
                            && data.payload.len() <= 200
                        {
                            return String::from_utf8(data.payload)
                                .map_err(|_| MeshError::Capabilities);
                        }
                    }
                }
            }
        };
        tokio::select! {biased;_=self.stop.cancelled()=>Err(MeshError::Cancelled),r=tokio::time::timeout(Duration::from_secs(config.radio.receive_timeout_seconds),receive)=>r.map_err(|_|MeshError::ReceiveTimeout)?}
    }
    pub async fn receive_event(&mut self) -> Result<RadioEvent, MeshError> {
        loop {
            let frame = tokio::select! {biased;_=self.stop.cancelled()=>return Err(MeshError::Cancelled),v=self.transport.receive()=>v?};
            let packet = match frame.payload_variant {
                Some(PayloadVariant::Packet(packet)) => packet,
                Some(PayloadVariant::NodeInfo(info)) => {
                    if update_node(&mut self.nodes, info, self.node_limit)? {
                        return Ok(RadioEvent::NodesChanged);
                    }
                    continue;
                }
                Some(PayloadVariant::Channel(mut channel)) => {
                    if let Some(settings) = channel.settings.as_mut() {
                        settings.psk.fill(0);
                    }
                    return Err(MeshError::Capabilities);
                }
                Some(
                    PayloadVariant::Config(_)
                    | PayloadVariant::ModuleConfig(_)
                    | PayloadVariant::Metadata(_)
                    | PayloadVariant::ConfigCompleteId(_),
                ) => return Err(MeshError::Capabilities),
                _ => continue,
            };
            if packet.from == self.own_node || !self.channels.contains(&packet.channel) {
                continue;
            }
            if let Some(protobufs::mesh_packet::PayloadVariant::Decoded(data)) =
                packet.payload_variant
            {
                if data.portnum == protobufs::PortNum::TextMessageApp as i32
                    && data.payload.len() <= 200
                {
                    let text =
                        String::from_utf8(data.payload).map_err(|_| MeshError::Capabilities)?;
                    return Ok(RadioEvent::Text(RadioText {
                        from: packet.from,
                        channel: packet.channel,
                        id: packet.id,
                        text,
                    }));
                }
            }
        }
    }
    pub async fn close(&mut self) -> Result<(), MeshError> {
        self.state = ConnectionState::Stopped;
        self.stop.cancel();
        self.transport.close().await
    }
}

/// Config and lock files must live outside source checkouts.
pub fn external_config(path: &Path) -> Result<(), MeshError> {
    let path = path.canonicalize().map_err(|_| MeshError::Configuration)?;
    if path.ancestors().any(|p| p.join(".git").exists()) {
        return Err(MeshError::Configuration);
    }
    Ok(())
}

#[cfg(test)]
mod node_tests {
    use super::*;

    #[test]
    fn directory_projects_only_bounded_labels_and_supports_updates_at_capacity() {
        let mut nodes = BTreeMap::new();
        let mut info = protobufs::NodeInfo {
            num: 1,
            user: Some(protobufs::User {
                long_name: "node-1\n".into(),
                short_name: "n1\0".into(),
                public_key: vec![7; 32],
                ..Default::default()
            }),
            position: Some(protobufs::Position::default()),
            ..Default::default()
        };
        assert!(update_node(&mut nodes, info.clone(), 1).is_ok_and(|changed| changed));
        let encoded = toml::to_string(&nodes[&1]).expect("sanitized node serialization");
        assert_eq!(nodes[&1].id, "!00000001");
        assert_eq!(nodes[&1].long_name, "node-1");
        assert_eq!(nodes[&1].short_name, "n1");
        assert!(!encoded.contains("public_key"));
        assert!(!encoded.contains("position"));
        assert!(update_node(&mut nodes, info.clone(), 1).is_ok_and(|changed| !changed));
        info.user.as_mut().expect("fixture user").long_name = "é".repeat(90);
        assert!(update_node(&mut nodes, info.clone(), 1).is_ok_and(|changed| changed));
        assert_eq!(nodes[&1].long_name.chars().count(), 64);
        info.num = 2;
        assert!(matches!(
            update_node(&mut nodes, info.clone(), 1),
            Err(MeshError::Capabilities)
        ));
        assert_eq!(nodes.len(), 1);
        info.num = 0;
        assert!(matches!(
            update_node(&mut nodes, info, 2),
            Err(MeshError::Capabilities)
        ));
        assert_eq!(nodes.len(), 1);
    }
}
