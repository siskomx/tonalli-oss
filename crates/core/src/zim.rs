//! ZIM content and title access over internal, removable or bounded pack sources.
use crate::pack::{PackError, PackSource};
use crate::title_index::{FoldedTitleTable, TitleIdentity};
use std::error::Error;
use std::io;
use std::ops::ControlFlow;
use std::path::Path;
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct TitleHit {
    pub title: String,
    pub url: String,
}
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct Article {
    pub title: String,
    pub url: String,
    pub mime_type: String,
    pub bytes: Vec<u8>,
}
/// Canonical content metadata; obtaining it never decodes the target body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntryInfo {
    pub title: String,
    pub url: String,
    pub mime_type: String,
    pub redirected: bool,
}
pub struct Zim {
    reader: zim_reader::Zim,
    source: Arc<dyn PackSource>,
    uuid: [u8; 16],
    checksum_pos: u64,
    titles: Option<FoldedTitleTable>,
    listing: zim_reader::ListingLocation,
}
struct Source(Arc<dyn PackSource>);
impl zim_reader::ReadAt for Source {
    fn len(&self) -> u64 {
        self.0.len()
    }
    fn read_at(&self, off: u64, bytes: &mut [u8]) -> io::Result<usize> {
        self.0.read_at(off, bytes).map_err(io::Error::other)
    }
}
impl Zim {
    pub(crate) fn index_source(&self) -> (Arc<dyn PackSource>, u64) {
        (Arc::clone(&self.source), self.checksum_pos)
    }

    pub(crate) fn canonical_article_entries(
        &self,
    ) -> impl Iterator<Item = Result<TitleHit, PackError>> + '_ {
        self.reader.iterate_by_urls().filter_map(|entry| {
            let entry = match entry {
                Ok(entry) => entry,
                Err(error) => return Some(Err(map_error(error))),
            };
            if !matches!(entry.namespace.as_byte(), b'A' | b'C')
                || !matches!(entry.target, Some(zim_reader::Target::Cluster(_, _)))
            {
                return None;
            }
            let zim_reader::MimeType::Type(mime) = &entry.mime_type else {
                return None;
            };
            let mime = mime.split(';').next().unwrap_or("").trim();
            if !["text/html", "application/xhtml+xml", "text/plain"]
                .iter()
                .any(|allowed| mime.eq_ignore_ascii_case(allowed))
            {
                return None;
            }
            Some(Ok(hit(&entry)))
        })
    }

    pub fn open(source: Arc<dyn PackSource>) -> Result<Self, PackError> {
        let mut header = [0; 80];
        read_exact(&*source, 0, &mut header)?;
        let checksum_pos = u64::from_le_bytes(
            header[72..80]
                .try_into()
                .map_err(|_| PackError::Truncated)?,
        );
        if checksum_pos
            .checked_add(16)
            .is_none_or(|n| n > source.len())
        {
            return Err(PackError::Truncated);
        }
        let mut uuid = [0; 16];
        uuid.copy_from_slice(&header[8..24]);
        let reader = zim_reader::Zim::from_store(zim_reader::Store::from_source(Arc::new(Source(
            source.clone(),
        ))))
        .map_err(map_error)?;
        let listing = reader.title_listing_location().map_err(map_error)?;
        Ok(Self {
            reader,
            source,
            uuid,
            checksum_pos,
            titles: None,
            listing,
        })
    }
    pub fn uuid(&self) -> [u8; 16] {
        self.uuid
    }
    pub fn stored_md5(&self) -> Result<[u8; 16], PackError> {
        let mut out = [0; 16];
        read_exact(&*self.source, self.checksum_pos, &mut out)?;
        Ok(out)
    }
    pub fn verify_md5(&self) -> Result<(), PackError> {
        self.reader.verify_checksum().map_err(map_error)
    }
    pub(crate) fn verify_md5_controlled<B>(
        &self,
        poll: impl FnMut(u64, u64) -> ControlFlow<B>,
    ) -> Result<ControlFlow<B>, PackError> {
        self.reader
            .verify_checksum_controlled(poll)
            .map_err(map_error)
    }
    pub fn title_count(&self) -> Result<usize, PackError> {
        self.listing()?.len().map_err(map_error)
    }
    /// Bounded archive display title; absent or unusable metadata has no title.
    /// Source/read errors remain typed so callers can exclude unavailable packs.
    pub fn metadata_title(&self) -> Result<Option<String>, PackError> {
        let Some(content) = self.reader.metadata("Title").map_err(map_error)? else {
            return Ok(None);
        };
        let length = content.len().map_err(map_error)?;
        if length > 512 {
            return Ok(None);
        }
        let bytes = content.read_range(0, length).map_err(map_error)?;
        Ok(readable_metadata_title(&bytes))
    }
    fn listing(&self) -> Result<zim_reader::Listing<'_>, PackError> {
        self.reader.listing_at(self.listing).map_err(map_error)
    }
    pub fn title_at(&self, position: u32) -> Result<TitleHit, PackError> {
        let index = self
            .listing()?
            .get(position as usize)
            .map_err(map_error)?
            .ok_or_else(|| PackError::NotFound(format!("title {position}")))?;
        let entry = self.reader.get_by_url_index(index).map_err(map_error)?;
        Ok(hit(&entry))
    }
    fn identity(&self) -> Result<TitleIdentity, PackError> {
        Ok(TitleIdentity {
            zim_uuid: self.uuid,
            zim_md5: self.stored_md5()?,
        })
    }
    pub fn build_title_index(&self, path: &Path) -> Result<(), PackError> {
        if self.titles.is_some() {
            return Err(PackError::Mismatch(
                "close indexed readers before rebuilding the title table".into(),
            ));
        }
        let count = self.title_count()?;
        FoldedTitleTable::build(
            path,
            self.identity()?,
            (0..count).filter_map(|p| {
                let result = (|| {
                    let position = u32::try_from(p)
                        .map_err(|_| PackError::Corrupt("title position overflow".into()))?;
                    let row = self.title_at(position)?;
                    Ok((position, row))
                })();
                match result {
                    Ok((position, row))
                        if row.url.starts_with("A/") || row.url.starts_with("C/") =>
                    {
                        Some(Ok((position, row.title)))
                    }
                    Ok(_) => None,
                    Err(error) => Some(Err(error)),
                }
            }),
        )?;
        Ok(())
    }
    pub fn with_title_index(mut self, path: &Path) -> Result<Self, PackError> {
        self.titles = Some(FoldedTitleTable::open(path, &self.identity()?)?);
        Ok(self)
    }
    pub fn suggest(&self, prefix: &str, limit: usize) -> Result<Vec<TitleHit>, PackError> {
        // Cached no-hit/empty queries must still observe removal of their archive.
        self.stored_md5()?;
        let prefix = prefix.trim();
        if prefix.is_empty() || limit == 0 {
            return Ok(Vec::new());
        }
        if let Some(table) = &self.titles {
            return table
                .suggest(prefix, limit)?
                .into_iter()
                .map(|p| self.title_at(p))
                .collect();
        }
        let count = self.title_count()?;
        let mut out = Vec::new();
        // Legacy listings are sorted by (namespace,title); v1 is the C article subset.
        for namespace in [b'A', b'C'] {
            let (mut low, mut high) = (0, count);
            while low < high {
                let mid = low + (high - low) / 2;
                let row = self.title_at(mid as u32)?;
                if (row.url.as_bytes()[0], row.title.as_bytes()) < (namespace, prefix.as_bytes()) {
                    low = mid + 1;
                } else {
                    high = mid;
                }
            }
            let mut found = 0;
            for position in low..count {
                let row = self.title_at(position as u32)?;
                if row.url.as_bytes()[0] != namespace || !row.title.starts_with(prefix) {
                    break;
                }
                out.push(row);
                found += 1;
                if found == limit {
                    break;
                }
            }
        }
        out.sort_by(|a, b| a.title.cmp(&b.title).then(a.url.cmp(&b.url)));
        out.truncate(limit);
        Ok(out)
    }
    fn lookup(&self, url: &str) -> Result<zim_reader::DirectoryEntry, PackError> {
        let (namespace, path) = match url.split_once('/') {
            Some((n, p)) if n.len() == 1 => (zim_reader::Namespace::from(n.as_bytes()[0]), p),
            _ => (zim_reader::Namespace::UserContent, url),
        };
        self.reader
            .get_by_path(namespace, path)
            .map_err(map_error)?
            .ok_or_else(|| PackError::NotFound(url.into()))
    }
    pub fn entry_info(&self, url: &str) -> Result<EntryInfo, PackError> {
        let entry = self.lookup(url)?;
        let redirected = matches!(entry.target.as_ref(), Some(zim_reader::Target::Redirect(_)));
        let entry = self.reader.resolve(entry).map_err(map_error)?;
        let hit = hit(&entry);
        Ok(EntryInfo {
            title: hit.title,
            url: hit.url,
            mime_type: entry_mime(&entry)?,
            redirected,
        })
    }
    pub fn article(&self, url: &str) -> Result<Article, PackError> {
        let entry = self.reader.resolve(self.lookup(url)?).map_err(map_error)?;
        let mime_type = entry_mime(&entry)?;
        let content = self
            .reader
            .entry_content(&entry)
            .map_err(map_error)?
            .ok_or_else(|| PackError::NotFound(url.into()))?;
        // Whole-body article responses are bounded; large indexes/resources need ranged serving.
        if content.len().map_err(map_error)? > 64 * 1024 * 1024 {
            return Err(PackError::Mismatch(
                "entry exceeds the 64 MiB whole-body read limit".into(),
            ));
        }
        let bytes = content.to_vec().map_err(map_error)?;
        let title = hit(&entry);
        Ok(Article {
            title: title.title,
            url: title.url,
            mime_type,
            bytes,
        })
    }
}
fn entry_mime(entry: &zim_reader::DirectoryEntry) -> Result<String, PackError> {
    match &entry.mime_type {
        zim_reader::MimeType::Type(mime) => Ok(mime.clone()),
        _ => Err(PackError::Corrupt("entry has no MIME content".into())),
    }
}
fn hit(entry: &zim_reader::DirectoryEntry) -> TitleHit {
    TitleHit {
        title: if entry.title.is_empty() {
            entry.url.clone()
        } else {
            entry.title.clone()
        },
        url: format!("{}/{}", entry.namespace.as_byte() as char, entry.url),
    }
}
fn readable_metadata_title(bytes: &[u8]) -> Option<String> {
    let title = std::str::from_utf8(bytes).ok()?.trim();
    if title.is_empty() || title.len() > 512 || title.chars().any(char::is_control) {
        None
    } else {
        Some(title.to_owned())
    }
}

fn read_exact(source: &dyn PackSource, off: u64, out: &mut [u8]) -> Result<(), PackError> {
    if off
        .checked_add(out.len() as u64)
        .is_none_or(|n| n > source.len())
    {
        return Err(PackError::Truncated);
    }
    let mut read = 0;
    while read < out.len() {
        let n = source.read_at(off + read as u64, &mut out[read..])?;
        if n == 0 || n > out.len() - read {
            return Err(PackError::Truncated);
        }
        read += n;
    }
    Ok(())
}
fn map_error(error: zim_reader::Error) -> PackError {
    let mut cause: &(dyn Error + 'static) = &error;
    loop {
        if let Some(pack) = cause.downcast_ref::<PackError>() {
            return pack.clone();
        }
        if let Some(io) = cause.downcast_ref::<io::Error>() {
            if let Some(pack) = io.get_ref().and_then(|e| e.downcast_ref::<PackError>()) {
                return pack.clone();
            }
            if io.kind() == io::ErrorKind::UnexpectedEof {
                return PackError::Truncated;
            }
        }
        match cause.source() {
            Some(next) => cause = next,
            None => break,
        }
    }
    match error {
        zim_reader::Error::InvalidChecksum => PackError::Checksum,
        zim_reader::Error::OutOfBounds | zim_reader::Error::MissingChecksum => PackError::Truncated,
        _ => PackError::Corrupt(error.to_string()),
    }
}

#[cfg(test)]
mod metadata_title_tests {
    use super::readable_metadata_title;

    #[test]
    fn metadata_title_is_bounded_plain_utf8_or_unusable() {
        assert_eq!(
            readable_metadata_title("  Niñez y educación\n".as_bytes()),
            Some("Niñez y educación".into())
        );
        assert_eq!(readable_metadata_title(&[0xff]), None);
        assert_eq!(readable_metadata_title(b" \t\n"), None);
        assert_eq!(readable_metadata_title(b"Title\0hidden"), None);
        assert_eq!(readable_metadata_title(b"Title\nsecond line"), None);
        assert_eq!(readable_metadata_title(&[b'a'; 513]), None);
        assert_eq!(readable_metadata_title(&[b'a'; 512]).unwrap().len(), 512);
    }
}
