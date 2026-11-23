use std::io::{Read, Write};
use std::net::IpAddr;
use std::path::Path;
use std::sync::Arc;

use rcgen::{CertificateParams, KeyPair, PublicKeyData};
use rustls::pki_types::PrivatePkcs8KeyDer;
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

use crate::pack::PackError;

// Fixed on purpose: short, typeable address. Two devices sharing at once is a name conflict.
pub(super) const HOSTNAME: &str = "tonalli.local";

pub(super) struct Identity {
    pub hostname: String,
    pub certificate_der: Vec<u8>,
    pub spki_sha256: String,
    pub config: Arc<rustls::ServerConfig>,
}

pub(super) fn load(directory: &Path, bind: IpAddr) -> Result<Identity, PackError> {
    let path = directory.join("server-key.pk8");
    let key = match std::fs::symlink_metadata(&path) {
        Ok(_) => read_key(&path)?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let generated = KeyPair::generate().map_err(crypto_error)?;
            let mut staged = tempfile::Builder::new()
                .prefix(".key-")
                .tempfile_in(directory)?;
            staged.write_all(generated.serialized_der())?;
            staged.as_file().sync_all()?;
            match staged.persist_noclobber(&path) {
                Ok(_) => generated,
                Err(error) if error.error.kind() == std::io::ErrorKind::AlreadyExists => {
                    read_key(&path)?
                }
                Err(error) => return Err(PackError::Io(error.error.to_string())),
            }
        }
        Err(error) => return Err(error.into()),
    };
    let spki_sha256 = format!("{:x}", Sha256::digest(key.subject_public_key_info()));
    let hostname = HOSTNAME.to_string();
    let mut names = vec![
        hostname.clone(),
        bind.to_string(),
        "127.0.0.1".into(),
        "::1".into(),
        "localhost".into(),
    ];
    names.sort();
    names.dedup();
    let certificate = CertificateParams::new(names)
        .map_err(crypto_error)?
        .self_signed(&key)
        .map_err(crypto_error)?;
    let certificate_der = certificate.der().to_vec();
    let config = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(
            vec![certificate_der.clone().into()],
            PrivatePkcs8KeyDer::from(key.serialize_der()).into(),
        )
        .map_err(crypto_error)?;
    Ok(Identity {
        hostname,
        certificate_der,
        spki_sha256,
        config: Arc::new(config),
    })
}

fn read_key(path: &Path) -> Result<KeyPair, PackError> {
    let info = std::fs::symlink_metadata(path)?;
    if !info.is_file() || info.file_type().is_symlink() || info.len() > 32768 {
        return Err(PackError::Mismatch(
            "LAN identity key must be a bounded ordinary file".into(),
        ));
    }
    let mut bytes = Zeroizing::new(Vec::new());
    std::fs::File::open(path)?
        .take(32769)
        .read_to_end(&mut bytes)?;
    if bytes.len() > 32768 {
        return Err(PackError::Mismatch("LAN identity key is oversized".into()));
    }
    KeyPair::from_pkcs8_der_and_sign_algo(
        &PrivatePkcs8KeyDer::from(bytes.as_slice()),
        &rcgen::PKCS_ECDSA_P256_SHA256,
    )
    .map_err(crypto_error)
}

fn crypto_error(error: impl std::fmt::Display) -> PackError {
    PackError::Corrupt(format!("LAN TLS identity: {error}"))
}
