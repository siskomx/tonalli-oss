//! Bounded PMTiles v3 reads over positioned sources, including removable media.

use std::io;
use std::sync::Arc;

use pmtiles_reader::{
    BackendResponse, Compression, Header, PmTilesReader, PmtError, PmtResult, SyncBackend,
    TileCoord,
};
use serde::Deserialize;

use crate::pack::{PackError, PackSource};

const MAX_COMPRESSED: usize = 16 * 1024 * 1024;

/// Header-derived map coverage plus the installed archive's attribution.
#[derive(Debug, Clone, PartialEq)]
pub struct TileMetadata {
    pub bounds: [f64; 4],
    pub min_zoom: u8,
    pub max_zoom: u8,
    pub center: [f64; 2],
    pub attribution: String,
}

/// No mmap or unbounded directory cache is used, regardless of the source path.
pub struct TileSource {
    reader: PmTilesReader<SourceBackend>,
    source: Arc<dyn PackSource>,
    metadata: TileMetadata,
}

impl TileSource {
    pub fn open(source: Arc<dyn PackSource>) -> Result<Self, PackError> {
        if source.len() < 127 {
            return Err(PackError::Truncated);
        }
        let backend = SourceBackend(Arc::clone(&source));
        let header_bytes = backend.read(0, 127).map_err(map_error)?.bytes;
        if header_bytes.len() != 127 {
            return Err(PackError::Truncated);
        }
        let header = Header::try_from_bytes(header_bytes).map_err(map_error)?;
        header
            .validate_source_length(source.len())
            .map_err(map_error)?;
        for codec in [header.internal_compression(), header.tile_compression] {
            if !matches!(codec, Compression::None | Compression::Gzip) {
                return Err(PackError::Corrupt(format!(
                    "unsupported PMTiles compression: {codec:?}"
                )));
            }
        }
        let reader = PmTilesReader::try_from_source(backend).map_err(map_error)?;
        let metadata_text = reader.get_metadata().map_err(map_error)?;
        // The fork caps decoded metadata at 1 MiB before this parser. Deserialize
        // only the needed field, skipping unknown values without building a Value tree.
        if !metadata_text.trim_start().starts_with('{') {
            return Err(PackError::Corrupt(
                "PMTiles metadata must be a JSON object".into(),
            ));
        }
        #[derive(Deserialize)]
        struct Attribution {
            #[serde(default)]
            attribution: String,
        }
        let attribution: Attribution = serde_json::from_str(&metadata_text)
            .map_err(|error| PackError::Corrupt(format!("PMTiles metadata: {error}")))?;
        let header = reader.get_header();
        let metadata = TileMetadata {
            bounds: [
                header.min_longitude,
                header.min_latitude,
                header.max_longitude,
                header.max_latitude,
            ],
            min_zoom: header.min_zoom,
            max_zoom: header.max_zoom,
            center: [header.center_longitude, header.center_latitude],
            attribution: attribution.attribution,
        };
        Ok(Self {
            reader,
            source,
            metadata,
        })
    }

    /// Returns decompressed format bytes, not the archive's compressed tile bytes.
    pub fn tile(&self, z: u8, x: u32, y: u32) -> Result<Option<Vec<u8>>, PackError> {
        // Even a root-cache miss must observe source removal on the next call.
        let mut probe = [0];
        if self.source.read_at(0, &mut probe)? != 1 {
            return Err(PackError::Truncated);
        }
        let coordinate = TileCoord::new(z, x, y).map_err(map_error)?;
        self.reader
            .get_tile_decompressed(coordinate)
            .map_err(map_error)?
            .map(|bytes| {
                let mut out = Vec::new();
                out.try_reserve_exact(bytes.len())
                    .map_err(|_| PackError::Corrupt("PMTiles tile allocation failed".into()))?;
                out.extend_from_slice(&bytes);
                Ok(out)
            })
            .transpose()
    }

    pub fn metadata(&self) -> &TileMetadata {
        &self.metadata
    }
}

struct SourceBackend(Arc<dyn PackSource>);
impl SyncBackend for SourceBackend {
    fn read(&self, offset: u64, length: usize) -> PmtResult<BackendResponse> {
        if length > MAX_COMPRESSED {
            return Err(PmtError::LimitExceeded);
        }
        let available = self
            .0
            .len()
            .checked_sub(offset)
            .ok_or_else(|| PmtError::Reading(io::Error::other(PackError::Truncated)))?;
        let length = length.min(usize::try_from(available).unwrap_or(usize::MAX));
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(length)
            .map_err(|_| PmtError::LimitExceeded)?;
        bytes.resize(length, 0);
        let mut done = 0;
        while done < length {
            let read = self
                .0
                .read_at(offset + done as u64, &mut bytes[done..])
                .map_err(|error| PmtError::Reading(io::Error::other(error)))?;
            if read == 0 {
                break;
            }
            if read > length - done {
                return Err(PmtError::InvalidEntry);
            }
            done += read;
        }
        bytes.truncate(done);
        Ok(BackendResponse::new(bytes.into()))
    }
}

fn map_error(error: PmtError) -> PackError {
    match error {
        PmtError::Truncated | PmtError::UnexpectedNumberOfBytesReturned(..) => PackError::Truncated,
        PmtError::Reading(error) => error
            .get_ref()
            .and_then(|inner| inner.downcast_ref::<PackError>())
            .cloned()
            .unwrap_or_else(|| PackError::Corrupt(error.to_string())),
        error => PackError::Corrupt(error.to_string()),
    }
}
