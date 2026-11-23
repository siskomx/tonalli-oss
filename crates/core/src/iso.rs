//! Read-only optical ISO/UDF images over positioned pack sources.

mod parser;
mod source;

use std::collections::BTreeMap;
use std::sync::Arc;

use crate::pack::{PackError, PackSource, WindowSource};

pub use source::ExtentSource;

/// File sections in logical order. UDF holes have no physical source bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Extent {
    Recorded { offset: u64, len: u64 },
    Zero { len: u64 },
}

impl Extent {
    pub fn len(&self) -> u64 {
        match self {
            Self::Recorded { len, .. } | Self::Zero { len } => *len,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Immutable bounded directory metadata; payloads remain in the original image.
pub struct IsoImage {
    source: Arc<dyn PackSource>,
    files: BTreeMap<String, Vec<Extent>>,
}

impl IsoImage {
    pub fn open(source: Arc<dyn PackSource>) -> Result<Self, PackError> {
        // Corrupt third-party parser input must not unwind through core callers.
        let files = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            parser::parse(Arc::clone(&source))
        }))
        .map_err(|_| PackError::Corrupt("optical metadata parser panicked".into()))??;
        Ok(Self { source, files })
    }

    pub fn find(&self, path: &str) -> Result<Vec<Extent>, PackError> {
        let path = normalized_path(path)?;
        source::probe(&*self.source)?;
        self.files
            .get(path)
            .cloned()
            .ok_or_else(|| PackError::NotFound(path.into()))
    }

    pub fn open_file(&self, path: &str) -> Result<Arc<dyn PackSource>, PackError> {
        let extents = self.find(path)?;
        if let [Extent::Recorded { offset, len }] = extents.as_slice() {
            Ok(Arc::new(WindowSource::new(
                Arc::clone(&self.source),
                *offset,
                *len,
            )?))
        } else {
            Ok(Arc::new(ExtentSource::new(
                Arc::clone(&self.source),
                extents,
            )?))
        }
    }
}

fn normalized_path(path: &str) -> Result<&str, PackError> {
    let path = path.strip_prefix('/').unwrap_or(path);
    if path.len() > 4096
        || path.contains(['\\', '\0'])
        || path.split('/').count() > 64
        || (!path.is_empty()
            && path
                .split('/')
                .any(|component| component.is_empty() || matches!(component, "." | "..")))
    {
        return Err(PackError::Mismatch("invalid optical image path".into()));
    }
    Ok(path)
}
