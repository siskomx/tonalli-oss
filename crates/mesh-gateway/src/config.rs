use crate::MeshError;
use serde::Deserialize;
use std::{
    fs::File,
    io::Read,
    path::{Path, PathBuf},
};

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub enabled: bool,
    pub radio: Radio,
    pub spike: Spike,
}

#[derive(Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Radio {
    pub transport: String,
    pub selector: String,
    pub lock_directory: PathBuf,
    pub baud: u32,
    pub dtr: bool,
    pub rts: bool,
    pub connect_timeout_seconds: u64,
    pub config_timeout_seconds: u64,
    pub receive_timeout_seconds: u64,
    pub max_nodes: usize,
    pub max_config_events: usize,
    pub accepted_firmware: Vec<String>,
    pub channel_index: Option<u32>,
}
impl Default for Radio {
    fn default() -> Self {
        Self {
            transport: "serial".into(),
            selector: String::new(),
            lock_directory: PathBuf::new(),
            baud: 115200,
            dtr: false,
            rts: false,
            connect_timeout_seconds: 10,
            config_timeout_seconds: 30,
            receive_timeout_seconds: 60,
            max_nodes: 1024,
            max_config_events: 8192,
            accepted_firmware: Vec::new(),
            channel_index: None,
        }
    }
}
#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Spike {
    pub text: Option<String>,
    pub show_test_text: bool,
}

impl Config {
    pub fn parse(source: &str) -> Result<Self, MeshError> {
        if source.len() > 65536 {
            return Err(MeshError::Configuration);
        }
        let config: Self = toml::from_str(source).map_err(|_| MeshError::Configuration)?;
        config.validate()?;
        Ok(config)
    }
    pub fn load(path: &Path) -> Result<Self, MeshError> {
        let mut source = String::new();
        File::open(path)
            .map_err(|_| MeshError::Configuration)?
            .take(65537)
            .read_to_string(&mut source)
            .map_err(|_| MeshError::Configuration)?;
        if source.len() > 65536 {
            return Err(MeshError::Configuration);
        }
        let mut config: Self = toml::from_str(&source).map_err(|_| MeshError::Configuration)?;
        for (key, value) in std::env::vars_os() {
            let Some(key) = key.to_str() else { continue };
            if !key.starts_with("TONALLI_MESH__") {
                continue;
            }
            let value = value.to_str().ok_or(MeshError::Configuration)?;
            macro_rules! number {
                ($field:expr) => {
                    $field = value.parse().map_err(|_| MeshError::Configuration)?
                };
            }
            match key {
                "TONALLI_MESH__ENABLED" => number!(config.enabled),
                "TONALLI_MESH__RADIO__TRANSPORT" => config.radio.transport = value.into(),
                "TONALLI_MESH__RADIO__SELECTOR" => config.radio.selector = value.into(),
                "TONALLI_MESH__RADIO__LOCK_DIRECTORY" => config.radio.lock_directory = value.into(),
                "TONALLI_MESH__RADIO__BAUD" => number!(config.radio.baud),
                "TONALLI_MESH__RADIO__DTR" => number!(config.radio.dtr),
                "TONALLI_MESH__RADIO__RTS" => number!(config.radio.rts),
                "TONALLI_MESH__RADIO__CONNECT_TIMEOUT_SECONDS" => {
                    number!(config.radio.connect_timeout_seconds)
                }
                "TONALLI_MESH__RADIO__CONFIG_TIMEOUT_SECONDS" => {
                    number!(config.radio.config_timeout_seconds)
                }
                "TONALLI_MESH__RADIO__RECEIVE_TIMEOUT_SECONDS" => {
                    number!(config.radio.receive_timeout_seconds)
                }
                "TONALLI_MESH__RADIO__MAX_NODES" => number!(config.radio.max_nodes),
                "TONALLI_MESH__RADIO__MAX_CONFIG_EVENTS" => number!(config.radio.max_config_events),
                "TONALLI_MESH__RADIO__ACCEPTED_FIRMWARE" => {
                    config.radio.accepted_firmware = value
                        .split(',')
                        .filter(|s| !s.is_empty())
                        .map(str::to_owned)
                        .collect()
                }
                "TONALLI_MESH__RADIO__CHANNEL_INDEX" => {
                    config.radio.channel_index =
                        Some(value.parse().map_err(|_| MeshError::Configuration)?)
                }
                "TONALLI_MESH__SPIKE__TEXT" => config.spike.text = Some(value.into()),
                "TONALLI_MESH__SPIKE__SHOW_TEST_TEXT" => number!(config.spike.show_test_text),
                _ => return Err(MeshError::Configuration),
            }
        }
        config.validate()?;
        Ok(config)
    }
    fn validate(&self) -> Result<(), MeshError> {
        let r = &self.radio;
        if !matches!(r.transport.as_str(), "serial" | "ble")
            || r.selector.len() > 4096
            || r.selector.chars().any(char::is_control)
            || r.baud == 0
            || r.baud > 4_000_000
            || [
                r.connect_timeout_seconds,
                r.config_timeout_seconds,
                r.receive_timeout_seconds,
            ]
            .iter()
            .any(|n| !(1..=300).contains(n))
            || !(1..=8192).contains(&r.max_nodes)
            || !(1..=65536).contains(&r.max_config_events)
            || r.accepted_firmware.len() > 32
            || r.accepted_firmware.iter().any(|v| {
                v.is_empty() || v.len() > 128 || !v.is_ascii() || v.chars().any(char::is_control)
            })
            || self
                .spike
                .text
                .as_ref()
                .is_some_and(|s| s.is_empty() || s.len() > 200 || s.contains('\0'))
            || (self.enabled && (r.selector.is_empty() || r.lock_directory.as_os_str().is_empty()))
            || self.spike.text.is_some()
                && (r.channel_index.is_none() || r.accepted_firmware.is_empty())
        {
            return Err(MeshError::Configuration);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_example_is_valid_and_unknown_values_are_not_echoed_in_errors() {
        assert!(Config::parse("enabled = false").is_ok());
        let error = Config::parse("unrecognized = 'PRIVATE_SENTINEL'")
            .err()
            .unwrap();
        assert_eq!(error, MeshError::Configuration);
        assert!(!format!("{error:?} {error}").contains("PRIVATE_SENTINEL"));
    }
}
