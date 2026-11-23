use std::fs::File;
use std::io::Read;
use std::path::Path;

use argon2::{Algorithm, Argon2, Block, Params, Version};
use chacha20poly1305::{
    aead::{Aead, KeyInit, Payload},
    XChaCha20Poly1305, XNonce,
};
use zeroize::Zeroizing;

use super::NotesError;

const MAGIC: &[u8; 8] = b"TONKEYS\0";
const VERSION: u32 = 1;
// Profile 1: Argon2id v19, 19 MiB, two passes, one lane, 32-byte output.
// This is a fixed spike profile; phone unlock time is not measured yet.
const PROFILE: u32 = 1;
const ENVELOPE_BYTES: usize = 104;
const HEADER_BYTES: usize = 56;

pub(super) fn random(bytes: &mut [u8]) -> Result<(), NotesError> {
    getrandom::fill(bytes)
        .map_err(|error| NotesError::Io(format!("system randomness failed: {error}")))
}

pub(super) fn wrap(passphrase: &str, key: &[u8; 32]) -> Result<[u8; ENVELOPE_BYTES], NotesError> {
    let mut bytes = [0; ENVELOPE_BYTES];
    bytes[..8].copy_from_slice(MAGIC);
    bytes[8..12].copy_from_slice(&VERSION.to_le_bytes());
    bytes[12..16].copy_from_slice(&PROFILE.to_le_bytes());
    random(&mut bytes[16..32])?;
    random(&mut bytes[32..56])?;
    let derived = derive(passphrase, &bytes[16..32])?;
    let cipher = XChaCha20Poly1305::new_from_slice(&derived[..]).map_err(|_| invalid())?;
    let nonce = XNonce::try_from(&bytes[32..56]).map_err(|_| invalid())?;
    let encrypted = cipher
        .encrypt(
            &nonce,
            Payload {
                msg: key,
                aad: &bytes[..HEADER_BYTES],
            },
        )
        .map_err(|_| invalid())?;
    bytes[HEADER_BYTES..].copy_from_slice(&encrypted);
    Ok(bytes)
}

pub(super) fn unwrap(path: &Path, passphrase: &str) -> Result<Zeroizing<[u8; 32]>, NotesError> {
    let metadata = std::fs::symlink_metadata(path).map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            NotesError::MissingKeys
        } else {
            error.into()
        }
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(invalid());
    }
    let mut file = File::open(path)?;
    if file.metadata()?.len() != ENVELOPE_BYTES as u64 {
        return Err(invalid());
    }
    let mut bytes = [0; ENVELOPE_BYTES];
    file.read_exact(&mut bytes)?;
    let mut extra = [0];
    if file.read(&mut extra)? != 0 {
        return Err(invalid());
    }
    if &bytes[..8] != MAGIC {
        return Err(invalid());
    }
    if bytes[8..12] != VERSION.to_le_bytes() || bytes[12..16] != PROFILE.to_le_bytes() {
        return Err(NotesError::Unsupported(
            "unknown notes envelope or Argon2 profile".into(),
        ));
    }
    let derived = derive(passphrase, &bytes[16..32])?;
    let cipher = XChaCha20Poly1305::new_from_slice(&derived[..]).map_err(|_| invalid())?;
    let nonce = XNonce::try_from(&bytes[32..56]).map_err(|_| invalid())?;
    let plaintext = Zeroizing::new(
        cipher
            .decrypt(
                &nonce,
                Payload {
                    msg: &bytes[HEADER_BYTES..],
                    aad: &bytes[..HEADER_BYTES],
                },
            )
            .map_err(|_| NotesError::UnlockFailed)?,
    );
    if plaintext.len() != 32 {
        return Err(invalid());
    }
    let mut key = Zeroizing::new([0; 32]);
    key.copy_from_slice(&plaintext);
    Ok(key)
}

fn derive(passphrase: &str, salt: &[u8]) -> Result<Zeroizing<[u8; 32]>, NotesError> {
    let params = Params::new(19 * 1024, 2, 1, Some(32)).map_err(|_| invalid())?;
    let mut blocks = Zeroizing::new(Vec::new());
    blocks
        .try_reserve_exact(params.block_count())
        .map_err(|_| NotesError::Io("Argon2 memory allocation failed".into()))?;
    blocks.resize(params.block_count(), Block::default());
    let argon = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let mut key = Zeroizing::new([0; 32]);
    argon
        .hash_password_into_with_memory(passphrase.as_bytes(), salt, &mut key[..], &mut blocks[..])
        .map_err(|_| invalid())?;
    Ok(key)
}

fn invalid() -> NotesError {
    NotesError::Corrupt("invalid notes key envelope".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::notes::{storage, Notes, Unlock};

    #[test]
    fn keys_sidecar_rewrite_survives_crash_before_rename() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("notes.db");
        let (notes, _) = Notes::create(&path, "old").unwrap();
        let key = unwrap(&path.with_extension("keys"), "old").unwrap();
        let before = std::fs::read(path.with_extension("keys")).unwrap();
        let staged =
            storage::stage(&path.with_extension("keys"), &wrap("new", &key).unwrap()).unwrap();
        drop(staged); // Same staging path as rotation, interrupted before persist.
        drop(notes);
        assert_eq!(std::fs::read(path.with_extension("keys")).unwrap(), before);
        assert!(Notes::open(&path, Unlock::Passphrase("old")).is_ok());
        assert!(matches!(
            Notes::open(&path, Unlock::Passphrase("new")),
            Err(NotesError::UnlockFailed)
        ));
    }
}
