use std::{net::SocketAddr, time::Duration};
pub(super) const GROUP: &str = "239.255.255.250:1900";
pub(super) const MAX_AGE: u64 = 1800;
pub(super) struct Search {
    pub target: String,
    pub wait: Duration,
}
pub(super) fn search(bytes: &[u8]) -> Option<Search> {
    if bytes.len() > 2048
        || !bytes
            .iter()
            .all(|b| matches!(b, b'\t' | b'\r' | b'\n' | 32..=126))
    {
        return None;
    }
    let message = std::str::from_utf8(bytes).ok()?.strip_suffix("\r\n\r\n")?;
    let mut lines = message.split("\r\n");
    if lines.next()? != "M-SEARCH * HTTP/1.1" {
        return None;
    }
    let mut headers = std::collections::HashMap::new();
    for line in lines {
        if line.len() > 512 || line.contains(['\r', '\n']) || headers.len() >= 16 {
            return None;
        }
        let (name, value) = line.split_once(':')?;
        if name.is_empty() || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') {
            return None;
        }
        if headers
            .insert(name.to_ascii_lowercase(), value.trim())
            .is_some()
        {
            return None;
        }
    }
    if headers.get("host") != Some(&GROUP) || headers.get("man") != Some(&"\"ssdp:discover\"") {
        return None;
    }
    let mx = *headers.get("mx")?;
    if mx.is_empty() || !mx.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let seconds = mx.parse::<u32>().ok()?;
    if seconds == 0 {
        return None;
    }
    let target = *headers.get("st")?;
    if target.is_empty() || target.len() > 128 || target.bytes().any(|b| b <= 32) {
        return None;
    }
    Some(Search {
        target: target.into(),
        wait: Duration::from_secs(u64::from(seconds.min(5))),
    })
}
pub(super) struct Advertisements {
    location: String,
    uuid: String,
    targets: [String; 5],
}
impl Advertisements {
    pub fn new(uuid: &str, address: SocketAddr) -> Self {
        let uuid = format!("uuid:{uuid}");
        Self {
            location: format!("http://{address}/description.xml"),
            targets: [
                "upnp:rootdevice".into(),
                uuid.clone(),
                "urn:schemas-upnp-org:device:MediaServer:1".into(),
                "urn:schemas-upnp-org:service:ContentDirectory:1".into(),
                "urn:schemas-upnp-org:service:ConnectionManager:1".into(),
            ],
            uuid,
        }
    }
    fn usn(&self, target: &str) -> String {
        if target == self.uuid {
            self.uuid.clone()
        } else {
            format!("{}::{target}", self.uuid)
        }
    }
    pub fn responses(&self, target: &str) -> Vec<String> {
        self.targets.iter().filter(|t| target=="ssdp:all" || target==t.as_str()).map(|target|format!("HTTP/1.1 200 OK\r\nCACHE-CONTROL: max-age={MAX_AGE}\r\nEXT:\r\nLOCATION: {}\r\nSERVER: {}/0 UPnP/1.0 Tonalli/0.1\r\nST: {target}\r\nUSN: {}\r\n\r\n",self.location,std::env::consts::OS,self.usn(target))).collect()
    }
    pub fn notifications(&self, alive: bool) -> Vec<String> {
        let details = if alive {
            format!("CACHE-CONTROL: max-age={MAX_AGE}\r\nLOCATION: {}\r\nSERVER: {}/0 UPnP/1.0 Tonalli/0.1\r\n",self.location,std::env::consts::OS)
        } else {
            String::new()
        };
        let kind = if alive { "ssdp:alive" } else { "ssdp:byebye" };
        self.targets.iter().map(|target|format!("NOTIFY * HTTP/1.1\r\nHOST: {GROUP}\r\n{details}NT: {target}\r\nNTS: {kind}\r\nUSN: {}\r\n\r\n",self.usn(target))).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const UUID: &str = "b512ca81-64d4-4380-b64b-0cec32b12fd2";
    fn request(target: &str, mx: &str) -> Vec<u8> {
        format!("M-SEARCH * HTTP/1.1\r\nHOST: {GROUP}\r\nMAN: \"ssdp:discover\"\r\nMX: {mx}\r\nST: {target}\r\n\r\n").into_bytes()
    }
    #[test]
    fn ssdp_accepts_bounded_search_and_caps_wait_at_five_seconds() {
        let value = search(&request("ssdp:all", "2")).expect("valid search");
        assert_eq!(value.target, "ssdp:all");
        assert_eq!(value.wait, Duration::from_secs(2));
        assert_eq!(
            search(&request("upnp:rootdevice", "99")).unwrap().wait,
            Duration::from_secs(5)
        );
        let lower = String::from_utf8(request("ssdp:all", "1"))
            .unwrap()
            .replace("HOST:", "host:")
            .replace("MAN:", "man:");
        assert!(search(lower.as_bytes()).is_some());
    }
    #[test]
    fn ssdp_rejects_ambiguous_malformed_and_amplifying_input() {
        let valid = String::from_utf8(request("ssdp:all", "1")).unwrap();
        for value in [
            valid.replace("M-SEARCH *", "GET /"),
            valid.replace("HTTP/1.1", "HTTP/1.0"),
            valid.replace(GROUP, "8.8.8.8:1900"),
            valid.replace("MX: 1\r\n", ""),
            valid.replace("MX: 1", "MX: 0"),
            valid.replace("MX: 1", "MX: -1"),
            valid.replace("MX: 1", "MX: 999999999999999999999"),
            valid.replace("MX: 1", "MX: 1\r\nmx: 2"),
            valid.replace("ST: ssdp:all", "ST: ssdp:all\r\nST: upnp:rootdevice"),
            valid.replace("\"ssdp:discover\"", "ssdp:discover"),
            valid.replace("\r\n", "\n"),
            format!("{valid}body"),
            valid.replace("ST: ssdp:all", "ST: ssdp:\0all"),
            valid.replace("ST: ssdp:all", &format!("ST: {}", "a".repeat(513))),
            valid.replace(
                "ST: ssdp:all",
                &format!("ST: ssdp:all\r\n{}", "X: a\r\n".repeat(32)),
            ),
        ] {
            assert!(
                search(value.as_bytes()).is_none(),
                "accepted malformed search"
            );
        }
        assert!(search(&vec![b'a'; 2049]).is_none());
        assert!(search(&[0xff]).is_none());
    }
    #[test]
    fn ssdp_publishes_all_five_identities_and_only_selected_http_origin() {
        let ad = Advertisements::new(UUID, "192.168.10.2:48901".parse().unwrap());
        let responses = ad.responses("ssdp:all");
        assert_eq!(responses.len(), 5);
        let targets = [
            "upnp:rootdevice".to_owned(),
            format!("uuid:{UUID}"),
            "urn:schemas-upnp-org:device:MediaServer:1".into(),
            "urn:schemas-upnp-org:service:ContentDirectory:1".into(),
            "urn:schemas-upnp-org:service:ConnectionManager:1".into(),
        ];
        for target in &targets {
            let one = ad.responses(target);
            assert_eq!(one.len(), 1);
            let value = &one[0];
            assert!(responses.contains(value));
            assert!(value.starts_with("HTTP/1.1 200 OK\r\n"));
            assert!(value.ends_with("\r\n\r\n"));
            assert!(value.contains(&format!("CACHE-CONTROL: max-age={MAX_AGE}\r\n")));
            assert!(value.contains("LOCATION: http://192.168.10.2:48901/description.xml\r\n"));
            assert!(value.contains("EXT:\r\n"));
            assert!(value.contains(&format!("ST: {target}\r\n")));
            let usn = if target.starts_with("uuid:") {
                target.clone()
            } else {
                format!("uuid:{UUID}::{target}")
            };
            assert!(value.contains(&format!("USN: {usn}\r\n")));
            assert!(!value.contains("/api/"));
        }
        for target in [
            "unknown",
            "urn:schemas-upnp-org:device:InternetGatewayDevice:1",
            "ssdp:all\r\nLOCATION: http://evil.invalid",
        ] {
            assert!(ad.responses(target).is_empty());
        }
        for alive in [true, false] {
            let values = ad.notifications(alive);
            assert_eq!(values.len(), 5);
            for value in values {
                assert!(value.starts_with("NOTIFY * HTTP/1.1\r\n"));
                assert!(value.contains(&format!("HOST: {GROUP}\r\n")));
                assert!(value.contains(if alive {
                    "NTS: ssdp:alive\r\n"
                } else {
                    "NTS: ssdp:byebye\r\n"
                }));
                assert!(value.contains("NT: "));
                assert!(!value.contains("ssdp:discover"));
            }
        }
    }
}
