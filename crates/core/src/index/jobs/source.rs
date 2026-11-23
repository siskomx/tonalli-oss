use super::*;
use crate::{catalog::CatalogKind, content::ManagedStore};
use sha2::{Digest, Sha256};

/// Created by native source resolution, never from a caller-supplied expected checksum or path.
#[derive(Clone)]
pub struct IndexSource {
    pub(super) id: PackId,
    pub(super) zim: Arc<Zim>,
    pub(super) meta: super::super::IndexMeta,
    pub(super) namespace: String,
    pub(super) binding: String,
}

impl IndexSource {
    pub fn registered(id: PackId, zim: Arc<Zim>) -> Result<Self, IndexJobError> {
        Self::bind(id, zim, "registered")
    }

    pub fn managed(store: &ManagedStore, id: &str) -> Result<Self, IndexJobError> {
        let (source, metadata) = store.open_source_with_metadata(id)?;
        if metadata.kind != CatalogKind::Zim {
            return Err(IndexJobError::InvalidRequest);
        }
        Self::bind(
            PackId::new(id)?,
            Arc::new(Zim::open(source)?),
            &format!("managed:{}", metadata.sha256),
        )
    }

    fn bind(id: PackId, zim: Arc<Zim>, authority: &str) -> Result<Self, IndexJobError> {
        let meta = super::super::identity(&zim)?;
        let namespace = namespace(id.as_str(), &meta, authority)?;
        Ok(Self {
            id,
            zim,
            meta,
            namespace,
            binding: authority.to_owned(),
        })
    }
}

pub(super) fn namespace(
    id: &str,
    meta: &super::super::IndexMeta,
    binding: &str,
) -> Result<String, IndexJobError> {
    let managed = binding.strip_prefix("managed:").is_some_and(|hash| {
        hash.len() == 64
            && hash
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    });
    if binding != "registered" && !managed {
        return Err(invalid());
    }
    let identity = serde_json::to_vec(&(id, meta, binding)).map_err(|_| invalid())?;
    Ok(format!("{:x}", Sha256::digest(identity)))
}
