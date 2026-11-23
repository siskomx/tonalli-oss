//! One-use delegation for an explicit native import, independent of reader Activity lifetime.
use super::{hex, valid_folder, MediaError, MediaStore};
use crate::{content::storage, download::write_atomic, pack::PackError};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::File,
    io::Read,
    time::{SystemTime, UNIX_EPOCH},
};
use subtle::ConstantTimeEq;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Grant {
    version: u32,
    hash: String,
    folder: String,
    expires: u64,
}
fn now() -> Result<u64, MediaError> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .map_err(|_| MediaError::Invalid)
}
impl MediaStore {
    fn grant(&self) -> Result<Option<Grant>, MediaError> {
        let file = match File::open(self.path("native-import.json")?) {
            Ok(file) => file,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e.into()),
        };
        let mut bytes = Vec::new();
        file.take(8193).read_to_end(&mut bytes)?;
        if bytes.len() > 8192 {
            return Err(MediaError::Catalog);
        }
        let grant: Option<Grant> =
            serde_json::from_slice(&bytes).map_err(|_| MediaError::Catalog)?;
        if grant.as_ref().is_some_and(|g| {
            g.version != 1 || !hex(&g.hash, 64) || !valid_folder(&g.folder) || g.expires < 900
        }) {
            return Err(MediaError::Catalog);
        }
        Ok(grant)
    }
    fn save_grant(&self, grant: Option<&Grant>) -> Result<(), MediaError> {
        let path = self.path("native-import.json")?;
        let bytes = serde_json::to_vec(&grant).map_err(|_| MediaError::Catalog)?;
        write_atomic(&path, &bytes)?;
        storage::durable(&path)?;
        Ok(())
    }
    /// Trusted native/Rust authority only; the HTTP caller must hold local Admin admission.
    pub fn authorize_native_import(&self, folder: &str) -> Result<String, MediaError> {
        if !valid_folder(folder) {
            return Err(MediaError::Invalid);
        }
        let _lock = storage::lock(&self.path("catalog.lock")?)?;
        self.grant()?;
        let mut random = [0; 32];
        getrandom::fill(&mut random).map_err(|e| PackError::Io(e.to_string()))?;
        let ticket: String = random.iter().map(|b| format!("{b:02x}")).collect();
        let grant = Grant {
            version: 1,
            hash: format!("{:x}", Sha256::digest(ticket.as_bytes())),
            folder: folder.into(),
            expires: now()?.checked_add(900).ok_or(MediaError::Invalid)?,
        };
        self.save_grant(Some(&grant))?;
        Ok(ticket)
    }
    pub fn claim_native_import(&self, ticket: &str) -> Result<String, MediaError> {
        if !hex(ticket, 64) {
            return Err(MediaError::Invalid);
        }
        let _lock = storage::lock(&self.path("catalog.lock")?)?;
        let grant = self.grant()?.ok_or(MediaError::Invalid)?;
        let hash = format!("{:x}", Sha256::digest(ticket.as_bytes()));
        let time = now()?;
        if time > grant.expires
            || time < grant.expires - 900
            || !bool::from(hash.as_bytes().ct_eq(grant.hash.as_bytes()))
        {
            return Err(MediaError::Invalid);
        }
        self.save_grant(None)?;
        Ok(grant.folder)
    }
}
