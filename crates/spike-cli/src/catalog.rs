//! Publisher tooling: private keys stay outside the checkout; only public pins ship.
use std::{
    fs::File,
    io::{Read, Write},
    path::Path,
};

use ring::{
    rand::SystemRandom,
    signature::{Ed25519KeyPair, KeyPair},
};
use tonalli_core::{catalog::Catalog, pack::PackError};
use zeroize::Zeroizing;

pub(crate) fn bounded_file(path: &Path, maximum: u64) -> Result<Vec<u8>, PackError> {
    crate::manifest::ensure_regular_or_absent(path)?;
    let mut bytes = Vec::new();
    File::open(path)?
        .take(maximum + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > maximum {
        return Err(PackError::Mismatch(
            "Publisher input exceeds its size bound".into(),
        ));
    }
    Ok(bytes)
}

pub(crate) fn publish_new(path: &Path, bytes: &[u8]) -> Result<(), PackError> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let mut staged = tempfile::NamedTempFile::new_in(parent)?;
    staged.write_all(bytes)?;
    staged.as_file().sync_all()?;
    let published = staged
        .persist_noclobber(path)
        .map_err(|error| PackError::from(error.error))?;
    let durable = || -> std::io::Result<()> {
        published.sync_all()?;
        #[cfg(unix)]
        File::open(parent)?.sync_all()?;
        Ok(())
    };
    durable().map_err(|error| PackError::PublicationUncertain(error.to_string()))
}

/// The parent directory must already be private. Never overwrites an existing key.
pub fn generate_key(path: &Path) -> Result<[u8; 32], PackError> {
    let document = Ed25519KeyPair::generate_pkcs8(&SystemRandom::new())
        .map_err(|_| PackError::Io("Publisher key generation failed".into()))?;
    let key = Ed25519KeyPair::from_pkcs8(document.as_ref())
        .map_err(|_| PackError::Corrupt("Invalid generated publisher key".into()))?;
    let public = key
        .public_key()
        .as_ref()
        .try_into()
        .map_err(|_| PackError::Corrupt("Invalid publisher public key".into()))?;
    publish_new(path, document.as_ref())?;
    Ok(public)
}

/// Sign the exact bounded JSON bytes after applying the same Core metadata rules as clients.
pub fn sign_catalog(input: &Path, private_key: &Path, output: &Path) -> Result<(), PackError> {
    let payload = bounded_file(input, 1024 * 1024)?;
    let secret = Zeroizing::new(bounded_file(private_key, 16 * 1024)?);
    let key = Ed25519KeyPair::from_pkcs8(&secret)
        .map_err(|_| PackError::Corrupt("Invalid publisher private key".into()))?;
    let signature = key.sign(&payload);
    Catalog::verify(&payload, signature.as_ref(), key.public_key().as_ref(), 0)?;
    publish_new(output, signature.as_ref())
}

pub fn run(arguments: &[String]) -> Result<(), PackError> {
    match arguments {
        [command, flag, destination] if command == "catalog-keygen" && flag == "--out" => {
            let public = generate_key(Path::new(destination))?;
            let hex: String = public.iter().map(|byte| format!("{byte:02x}")).collect();
            println!("{}", serde_json::json!({"publisher_public_key":hex}));
            Ok(())
        }
        [command, catalog_flag, input, key_flag, key, out_flag, output]
            if command == "catalog-sign" && catalog_flag == "--catalog" && key_flag == "--key" && out_flag == "--out" => {
            sign_catalog(Path::new(input), Path::new(key), Path::new(output))?;
            println!("Signed catalog; private key and input were preserved.");
            Ok(())
        }
        _ => Err(PackError::Mismatch("catalog-keygen --out NEW_PRIVATE_KEY | catalog-sign --catalog JSON --key PRIVATE_KEY --out NEW_SIGNATURE".into())),
    }
}
