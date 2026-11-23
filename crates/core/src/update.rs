//! Opt-in app updates. A release publishes a manifest plus a detached Ed25519 signature; nothing
//! in it is trusted until the signature verifies against a public key the user configured.

use std::cmp::Ordering;
use std::str::FromStr;

use ring::signature::{UnparsedPublicKey, ED25519};
use serde::Deserialize;

use crate::pack::PackError;

mod package;
mod store;
pub use package::unpack;
pub use store::{Ready, Settings, Store};

const MAX_MANIFEST_BYTES: usize = 256 * 1024;
const MAX_ASSET_BYTES: u64 = 2 << 30;
const MAX_NOTES_BYTES: usize = 16 * 1024;
const MAX_ASSETS: usize = 16;
const MAX_URL_BYTES: usize = 2048;
const SCHEMA_VERSION: u32 = 1;

fn refused(reason: &str) -> PackError {
    PackError::Mismatch(format!("update manifest: {reason}"))
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Identifier {
    Numeric(u64),
    Text(String),
}

impl Ord for Identifier {
    fn cmp(&self, other: &Self) -> Ordering {
        match (self, other) {
            (Self::Numeric(a), Self::Numeric(b)) => a.cmp(b),
            (Self::Numeric(_), Self::Text(_)) => Ordering::Less,
            (Self::Text(_), Self::Numeric(_)) => Ordering::Greater,
            (Self::Text(a), Self::Text(b)) => a.cmp(b),
        }
    }
}

impl PartialOrd for Identifier {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// `MAJOR.MINOR.PATCH` with an optional `-pre.release` part, ordered like semantic versions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Version {
    core: [u64; 3],
    pre: Vec<Identifier>,
}

impl Ord for Version {
    fn cmp(&self, other: &Self) -> Ordering {
        self.core.cmp(&other.core).then_with(|| {
            match (self.pre.is_empty(), other.pre.is_empty()) {
                (true, true) => Ordering::Equal,
                (true, false) => Ordering::Greater,
                (false, true) => Ordering::Less,
                (false, false) => self.pre.cmp(&other.pre),
            }
        })
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl FromStr for Version {
    type Err = PackError;

    fn from_str(text: &str) -> Result<Self, PackError> {
        let invalid = || refused("invalid version");
        let text = text.strip_prefix('v').unwrap_or(text);
        if text.len() > 64 {
            return Err(invalid());
        }
        let (core, pre) = match text.split_once('-') {
            Some((core, pre)) => (core, Some(pre)),
            None => (text, None),
        };
        let numbers = core
            .split('.')
            .map(|part| {
                let digits = !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit());
                digits.then(|| part.parse::<u64>().ok()).flatten()
            })
            .collect::<Option<Vec<_>>>()
            .ok_or_else(invalid)?;
        let core: [u64; 3] = numbers.try_into().map_err(|_| invalid())?;
        let pre = match pre {
            None => Vec::new(),
            Some(pre) => pre
                .split('.')
                .map(|part| {
                    if part.is_empty()
                        || !part
                            .bytes()
                            .all(|b| b.is_ascii_alphanumeric() || b == b'-')
                    {
                        return None;
                    }
                    Some(if part.bytes().all(|b| b.is_ascii_digit()) {
                        Identifier::Numeric(part.parse().ok()?)
                    } else {
                        Identifier::Text(part.to_owned())
                    })
                })
                .collect::<Option<Vec<_>>>()
                .ok_or_else(invalid)?,
        };
        Ok(Self { core, pre })
    }
}

/// The release tag that holds only the manifest. A fixed tag works for pre-releases too, which the
/// `releases/latest` shortcut skips.
pub const FEED_TAG: &str = "update-feed";
pub const MANIFEST_FILE: &str = "update-manifest.json";
pub const SIGNATURE_FILE: &str = "update-manifest.sig";

fn repository_part(part: &str) -> bool {
    !part.is_empty()
        && part != "."
        && part != ".."
        && part.len() <= 100
        && part
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
}

fn github_folder(owner: &str, repository: &str, tag: &str) -> Result<String, PackError> {
    let repository = repository.strip_suffix(".git").unwrap_or(repository);
    if !repository_part(owner) || !repository_part(repository) || !repository_part(tag) {
        return Err(refused("invalid repository name"));
    }
    Ok(format!("https://github.com/{owner}/{repository}/releases/download/{tag}/"))
}

/// Turns what a person typed (`owner/repo`, a GitHub page, or an https folder) into the folder that
/// holds the manifest. Only public https domains are accepted.
pub fn normalize_source(input: &str) -> Result<String, PackError> {
    let input = input.trim();
    let invalid = || refused("update source must be owner/repo or an https address");
    if input.is_empty() || input.len() > MAX_URL_BYTES {
        return Err(invalid());
    }
    let address = if input.contains("://") {
        input.to_owned()
    } else if input.starts_with("github.com/") {
        format!("https://{input}")
    } else {
        return match input.split('/').collect::<Vec<_>>().as_slice() {
            [owner, repository] => github_folder(owner, repository, FEED_TAG),
            _ => Err(invalid()),
        };
    };
    let parsed = url::Url::parse(&address).map_err(|_| invalid())?;
    let domain = match parsed.host() {
        Some(url::Host::Domain(domain)) if domain.contains('.') => domain.to_owned(),
        _ => return Err(invalid()),
    };
    if parsed.scheme() != "https"
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
    {
        return Err(invalid());
    }
    let segments: Vec<&str> = parsed
        .path_segments()
        .map(|segments| segments.filter(|segment| !segment.is_empty()).collect())
        .unwrap_or_default();
    if matches!(domain.as_str(), "github.com" | "www.github.com") {
        return match segments.as_slice() {
            [owner, repository, "releases", "download", tag, ..] => github_folder(owner, repository, tag),
            [owner, repository, ..] => github_folder(owner, repository, FEED_TAG),
            _ => Err(invalid()),
        };
    }
    let mut folder = format!("https://{domain}");
    if let Some(port) = parsed.port() {
        folder.push_str(&format!(":{port}"));
    }
    folder.push('/');
    for segment in segments {
        folder.push_str(segment);
        folder.push('/');
    }
    Ok(folder)
}

/// The publisher key the user trusts: 32 bytes as 64 hex characters.
pub fn parse_public_key(input: &str) -> Result<[u8; 32], PackError> {
    let input = input.trim();
    let invalid = || refused("trusted key must be 64 hexadecimal characters");
    if input.len() != 64 || !input.is_ascii() {
        return Err(invalid());
    }
    let mut key = [0u8; 32];
    for (slot, pair) in key.iter_mut().zip(input.as_bytes().chunks(2)) {
        let digits = std::str::from_utf8(pair).map_err(|_| invalid())?;
        *slot = u8::from_str_radix(digits, 16).map_err(|_| invalid())?;
    }
    Ok(key)
}

/// The running app's release version, set in the workspace manifest.
pub fn app_version() -> Result<Version, PackError> {
    env!("CARGO_PKG_VERSION").parse()
}

/// The asset name a build downloads, or `None` for platforms without updates.
pub fn current_platform() -> Option<&'static str> {
    if cfg!(target_os = "android") {
        Some("android-arm64")
    } else if cfg!(target_os = "ios") {
        Some("ios-arm64")
    } else if cfg!(all(windows, target_arch = "x86_64")) {
        Some("windows-x64")
    } else {
        None
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Asset {
    pub platform: String,
    pub url: String,
    pub bytes: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub schema_version: u32,
    version: String,
    pub published: String,
    pub notes: String,
    pub page: String,
    pub assets: Vec<Asset>,
}

fn https(url: &str) -> bool {
    url.len() <= MAX_URL_BYTES
        && url::Url::parse(url)
            .is_ok_and(|parsed| parsed.scheme() == "https" && parsed.host_str().is_some())
}

impl Manifest {
    /// Signature first, then the bounded parse, so unauthenticated bytes are never interpreted.
    pub fn verify(payload: &[u8], signature: &[u8], public_key: &[u8]) -> Result<Self, PackError> {
        if payload.len() > MAX_MANIFEST_BYTES {
            return Err(refused("too large"));
        }
        if public_key.len() != 32 || signature.len() != 64 {
            return Err(refused("signature or key has the wrong length"));
        }
        UnparsedPublicKey::new(&ED25519, public_key)
            .verify(payload, signature)
            .map_err(|_| PackError::Checksum)?;
        let manifest: Manifest = serde_json::from_slice(payload)
            .map_err(|_| PackError::Corrupt("update manifest is not valid".into()))?;
        manifest.validate()?;
        Ok(manifest)
    }

    fn validate(&self) -> Result<(), PackError> {
        if self.schema_version != SCHEMA_VERSION {
            return Err(refused("unsupported schema version"));
        }
        self.version.parse::<Version>()?;
        if self.notes.len() > MAX_NOTES_BYTES || self.published.len() > 32 {
            return Err(refused("text field too long"));
        }
        if !self.page.is_empty() && !https(&self.page) {
            return Err(refused("release page must be https"));
        }
        if self.assets.is_empty() || self.assets.len() > MAX_ASSETS {
            return Err(refused("asset count out of range"));
        }
        for (index, asset) in self.assets.iter().enumerate() {
            let hex = asset.sha256.len() == 64
                && asset
                    .sha256
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
            if !https(&asset.url)
                || !hex
                || asset.bytes == 0
                || asset.bytes > MAX_ASSET_BYTES
                || asset.platform.is_empty()
                || asset.platform.len() > 32
                || self.assets[..index]
                    .iter()
                    .any(|earlier| earlier.platform == asset.platform)
            {
                return Err(refused("invalid asset"));
            }
        }
        Ok(())
    }

    pub fn version(&self) -> Version {
        // Checked in `validate`, the only way to obtain a Manifest.
        self.version
            .parse()
            .unwrap_or_else(|_| Version { core: [0; 3], pre: Vec::new() })
    }

    pub fn version_text(&self) -> &str {
        &self.version
    }

    pub fn asset_for(&self, platform: &str) -> Option<&Asset> {
        self.assets.iter().find(|asset| asset.platform == platform)
    }

    pub fn is_newer_than(&self, installed: &Version) -> bool {
        self.version() > *installed
    }
}
