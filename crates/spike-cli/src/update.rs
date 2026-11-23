//! Publisher tooling for app updates: builds and signs `update-manifest.json` for one release.
//! Private keys stay outside the checkout; the apps only ever see the public key and signature.
use std::{
    fs::File,
    io::Read,
    path::{Path, PathBuf},
};

use ring::signature::{Ed25519KeyPair, KeyPair};
use sha2::{Digest, Sha256};
use tonalli_core::{
    pack::PackError,
    update::{Manifest, MANIFEST_FILE, SIGNATURE_FILE},
};
use zeroize::Zeroizing;

use crate::catalog::{bounded_file, publish_new};

const PLATFORMS: [&str; 3] = ["android-arm64", "windows-x64", "ios-arm64"];

pub struct Release {
    pub version: String,
    pub published: String,
    pub notes: String,
    pub page: String,
    pub base_url: String,
    /// `(platform, package file)`; the file name becomes the last part of the download address.
    pub assets: Vec<(String, PathBuf)>,
}

fn refused(reason: &str) -> PackError {
    PackError::Mismatch(format!("update release: {reason}"))
}

fn digest(path: &Path) -> Result<(u64, String), PackError> {
    crate::manifest::ensure_regular_or_absent(path)?;
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 1 << 20];
    let mut total = 0u64;
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        total += read as u64;
    }
    let hex = hasher.finalize().iter().map(|byte| format!("{byte:02x}")).collect();
    Ok((total, hex))
}

fn file_name(path: &Path) -> Result<&str, PackError> {
    path.file_name()
        .and_then(|name| name.to_str())
        .filter(|name| {
            !name.is_empty()
                && name
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
        })
        .ok_or_else(|| refused("package file names may only use letters, digits, . _ and -"))
}

/// Writes `update-manifest.json` and `update-manifest.sig` into an existing folder. Refuses to
/// overwrite either, and checks the result with the same verifier the apps use.
pub fn build_manifest(
    release: &Release,
    private_key: &Path,
    out_dir: &Path,
) -> Result<(), PackError> {
    let json_path = out_dir.join(MANIFEST_FILE);
    let signature_path = out_dir.join(SIGNATURE_FILE);
    if json_path.exists() || signature_path.exists() {
        return Err(refused("outputs already exist and are never overwritten"));
    }
    if !release.base_url.starts_with("https://") {
        return Err(refused("base address must be https"));
    }
    let base = if release.base_url.ends_with('/') {
        release.base_url.clone()
    } else {
        format!("{}/", release.base_url)
    };
    let mut assets = Vec::new();
    for (platform, path) in &release.assets {
        if !PLATFORMS.contains(&platform.as_str()) {
            return Err(refused("unknown platform"));
        }
        let (bytes, sha256) = digest(path)?;
        assets.push(serde_json::json!({
            "platform": platform,
            "url": format!("{base}{}", file_name(path)?),
            "bytes": bytes,
            "sha256": sha256,
        }));
    }
    let payload = serde_json::to_vec(&serde_json::json!({
        "schema_version": 1,
        "version": release.version,
        "published": release.published,
        "notes": release.notes,
        "page": release.page,
        "assets": assets,
    }))
    .map_err(|_| refused("could not write the manifest"))?;

    let secret = Zeroizing::new(bounded_file(private_key, 16 * 1024)?);
    let key = Ed25519KeyPair::from_pkcs8(&secret)
        .map_err(|_| PackError::Corrupt("Invalid publisher private key".into()))?;
    let signature = key.sign(&payload);
    Manifest::verify(&payload, signature.as_ref(), key.public_key().as_ref())?;

    publish_new(&json_path, &payload)?;
    if let Err(error) = publish_new(&signature_path, signature.as_ref()) {
        let _ = std::fs::remove_file(&json_path);
        return Err(error);
    }
    Ok(())
}

fn value<'a>(arguments: &'a [String], flag: &str) -> Result<&'a str, PackError> {
    arguments
        .iter()
        .position(|argument| argument == flag)
        .and_then(|index| arguments.get(index + 1))
        .map(String::as_str)
        .ok_or_else(|| usage())
}

fn usage() -> PackError {
    PackError::Mismatch(
        "update-keygen --out NEW_PRIVATE_KEY | update-manifest --version V --published YYYY-MM-DD [--notes-file F] --page URL --base-url URL --key PRIVATE_KEY --out-dir EXISTING_DIR --asset PLATFORM=FILE..."
            .into(),
    )
}

pub fn run(arguments: &[String]) -> Result<(), PackError> {
    match arguments.first().map(String::as_str) {
        Some("update-keygen") => {
            let destination = value(arguments, "--out")?;
            let public = crate::catalog::generate_key(Path::new(destination))?;
            let hex: String = public.iter().map(|byte| format!("{byte:02x}")).collect();
            println!("{}", serde_json::json!({"update_public_key": hex}));
            Ok(())
        }
        Some("update-manifest") => {
            let notes = match arguments.iter().position(|argument| argument == "--notes-file") {
                Some(index) => {
                    let path = arguments.get(index + 1).ok_or_else(usage)?;
                    String::from_utf8(bounded_file(Path::new(path), 16 * 1024)?)
                        .map_err(|_| refused("notes must be UTF-8 text"))?
                }
                None => String::new(),
            };
            let mut assets = Vec::new();
            for (index, argument) in arguments.iter().enumerate() {
                if argument == "--asset" {
                    let pair = arguments.get(index + 1).ok_or_else(usage)?;
                    let (platform, path) = pair.split_once('=').ok_or_else(usage)?;
                    assets.push((platform.to_owned(), PathBuf::from(path)));
                }
            }
            let release = Release {
                version: value(arguments, "--version")?.to_owned(),
                published: value(arguments, "--published")?.to_owned(),
                notes,
                page: value(arguments, "--page")?.to_owned(),
                base_url: value(arguments, "--base-url")?.to_owned(),
                assets,
            };
            build_manifest(
                &release,
                Path::new(value(arguments, "--key")?),
                Path::new(value(arguments, "--out-dir")?),
            )?;
            println!("Signed update manifest {}; private key and packages were preserved.", release.version);
            Ok(())
        }
        _ => Err(usage()),
    }
}
