//! Window-less sharing support. Core still owns TLS, pairing and content isolation.
use std::{
    io::Write,
    net::IpAddr,
    path::{Path, PathBuf},
};

use serde_json::{json, Value};

/// A restarted run must not be stopped by a request left over from the previous one.
pub fn stop_path(status: &Path) -> PathBuf {
    let mut name = status.as_os_str().to_owned();
    name.push(".stop");
    PathBuf::from(name)
}

pub fn choose_address(choices: &[IpAddr], requested: Option<IpAddr>) -> Result<IpAddr, String> {
    match (requested, choices) {
        (Some(address), choices) if choices.contains(&address) => Ok(address),
        (Some(_), _) => Err("The chosen address is not a private LAN address on this computer".into()),
        (None, [only]) => Ok(*only),
        (None, []) => Err("No private LAN address is available".into()),
        (None, _) => Err("Several LAN addresses are available; name one with --headless".into()),
    }
}

pub struct Report<'a> {
    pub pid: u32,
    pub running: bool,
    pub url: Option<&'a str>,
    pub pairing_code: Option<&'a str>,
    pub devices: usize,
    pub error: Option<&'a str>,
    pub updated: u64,
}

pub fn status_value(report: &Report) -> Value {
    json!({
        "pid": report.pid,
        "running": report.running,
        "url": report.url,
        "pairing_code": report.pairing_code,
        "devices": report.devices,
        "error": report.error,
        "updated": report.updated,
    })
}

/// Replaces the file in one step, so a reader never sees a partial document.
pub fn write_status(path: &Path, value: &Value) -> std::io::Result<()> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent)?;
    let mut staged = tempfile::NamedTempFile::new_in(parent)?;
    staged.write_all(value.to_string().as_bytes())?;
    staged.as_file().sync_all()?;
    staged.persist(path).map_err(|failure| failure.error)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(value: &str) -> IpAddr {
        value.parse().unwrap()
    }

    #[test]
    fn a_requested_address_must_be_a_current_private_choice() {
        let choices = [ip("192.168.1.20"), ip("10.0.0.5")];
        assert_eq!(choose_address(&choices, Some(ip("10.0.0.5"))), Ok(ip("10.0.0.5")));
        assert!(choose_address(&choices, Some(ip("192.168.1.99"))).is_err());
        assert!(choose_address(&choices, Some(ip("0.0.0.0"))).is_err());
    }

    #[test]
    fn auto_accepts_exactly_one_choice() {
        assert_eq!(choose_address(&[ip("192.168.1.20")], None), Ok(ip("192.168.1.20")));
        assert!(choose_address(&[], None).is_err());
        assert!(choose_address(&[ip("192.168.1.20"), ip("10.0.0.5")], None).is_err());
    }

    #[test]
    fn stop_request_sits_beside_the_status_file() {
        assert_eq!(stop_path(Path::new("C:/private/status.json")), PathBuf::from("C:/private/status.json.stop"));
        assert_eq!(stop_path(Path::new("status")), PathBuf::from("status.stop"));
    }

    #[test]
    fn status_is_replaced_whole_and_carries_only_sharing_facts() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("nested/status.json");
        let report = |running, devices| Report {
            pid: 42,
            running,
            url: Some("https://192.168.1.20:4000/pair"),
            pairing_code: Some("123456"),
            devices,
            error: None,
            updated: 7,
        };
        write_status(&path, &status_value(&report(true, 1))).unwrap();
        write_status(&path, &status_value(&report(false, 0))).unwrap();
        let value: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(value["running"], false);
        assert_eq!(value["devices"], 0);
        assert_eq!(value["pid"], 42);
        let mut keys: Vec<_> = value.as_object().unwrap().keys().cloned().collect();
        keys.sort();
        assert_eq!(keys, ["devices", "error", "pairing_code", "pid", "running", "updated", "url"]);
        assert_eq!(std::fs::read_dir(path.parent().unwrap()).unwrap().count(), 1);
    }
}
