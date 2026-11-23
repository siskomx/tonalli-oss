//! Bounded EPUB/CBZ reads over immutable positioned sources; never extract paths.
use crate::pack::{PackError, PackSource};
use quick_xml::{encoding::DecodingReader, events::Event, Reader, XmlVersion};
use serde::Serialize;
use std::{
    cmp::Ordering,
    collections::HashMap,
    io::{self, Read, Seek, SeekFrom},
    sync::Arc,
};
use zip::{
    read::{ArchiveOffset, Config},
    result::ZipError,
    CompressionMethod, ZipArchive,
};

const MAX_FILES: usize = 16_384;
const MAX_MEMBER: u64 = 32 * 1024 * 1024;
const MAX_XML: u64 = 1024 * 1024;
const MAX_EXPANDED: u64 = 1024 * 1024 * 1024;
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BookKind {
    Epub,
    Comic,
}
#[derive(Clone, Debug, Serialize)]
pub struct Chapter {
    pub path: String,
    pub title: String,
}
#[derive(Clone, Debug, Serialize)]
pub struct BookMetadata {
    pub kind: BookKind,
    pub chapters: Vec<Chapter>,
}
pub struct Resource {
    pub mime: &'static str,
    pub bytes: Vec<u8>,
}
pub struct Book {
    archive: Archive,
    metadata: BookMetadata,
}
pub(crate) struct Archive {
    archive: ZipArchive<Positioned>,
    members: HashMap<String, usize>,
}
#[derive(Clone)]
struct Positioned {
    source: Arc<dyn PackSource>,
    offset: u64,
    budget: u64,
}
impl Read for Positioned {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        let count = bytes.len().min(
            self.source
                .len()
                .saturating_sub(self.offset)
                .min(usize::MAX as u64) as usize,
        );
        if count as u64 > self.budget {
            return Err(io::Error::other(limit()));
        }
        let actual = self
            .source
            .read_at(self.offset, &mut bytes[..count])
            .map_err(io::Error::other)?;
        if actual > count {
            return Err(io::Error::other(corrupt()));
        }
        self.offset = self
            .offset
            .checked_add(actual as u64)
            .ok_or_else(|| io::Error::other(corrupt()))?;
        self.budget -= actual as u64;
        Ok(actual)
    }
}
impl Seek for Positioned {
    fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
        let next = match from {
            SeekFrom::Start(value) => value as i128,
            SeekFrom::End(value) => self.source.len() as i128 + value as i128,
            SeekFrom::Current(value) => self.offset as i128 + value as i128,
        };
        self.offset = u64::try_from(next).map_err(|_| io::Error::other(corrupt()))?;
        Ok(self.offset)
    }
}
fn corrupt() -> PackError {
    PackError::Corrupt("Invalid book archive".into())
}
fn limit() -> PackError {
    PackError::Mismatch("This book exceeds supported archive limits".into())
}
fn zip_error(error: ZipError) -> PackError {
    match error {
        ZipError::Io(error) => {
            if let Some(problem) = error
                .get_ref()
                .and_then(|inner| inner.downcast_ref::<PackError>())
            {
                return problem.clone();
            }
            match error.kind() {
                io::ErrorKind::InvalidData => corrupt(),
                io::ErrorKind::UnexpectedEof => PackError::Truncated,
                _ => PackError::Io(error.to_string()),
            }
        }
        ZipError::FileNotFound => PackError::NotFound("Book resource".into()),
        _ => corrupt(),
    }
}
fn read_exact(source: &dyn PackSource, offset: u64, bytes: &mut [u8]) -> Result<(), PackError> {
    let mut filled = 0;
    while filled < bytes.len() {
        let count = source.read_at(
            offset.checked_add(filled as u64).ok_or_else(corrupt)?,
            &mut bytes[filled..],
        )?;
        if count == 0 {
            return Err(PackError::Truncated);
        }
        if count > bytes.len() - filled {
            return Err(corrupt());
        }
        filled += count;
    }
    Ok(())
}
// Bound directory allocations before handing hostile counts to the ZIP parser.
fn preflight(source: &dyn PackSource) -> Result<(), PackError> {
    if source.len() < 22 || source.len() > 2 * 1024 * 1024 * 1024 {
        return Err(limit());
    }
    let count = source.len().min(65_557) as usize;
    let offset = source.len() - count as u64;
    let mut tail = vec![0; count];
    read_exact(source, offset, &mut tail)?;
    let end = (0..=count - 22)
        .rev()
        .find(|&index| {
            tail[index..].starts_with(b"PK\x05\x06")
                && index + 22 + u16::from_le_bytes([tail[index + 20], tail[index + 21]]) as usize
                    == count
        })
        .ok_or_else(corrupt)?;
    let eocd = &tail[end..];
    let short = |at: usize| u16::from_le_bytes([eocd[at], eocd[at + 1]]) as u64;
    let long =
        |at: usize| u32::from_le_bytes([eocd[at], eocd[at + 1], eocd[at + 2], eocd[at + 3]]) as u64;
    if short(4) != 0
        || short(6) != 0
        || short(8) != short(10)
        || short(10) == 0
        || short(10) > MAX_FILES as u64
        || long(12) > 16 * 1024 * 1024
    {
        return Err(limit());
    }
    let absolute = offset + end as u64;
    if long(16).checked_add(long(12)) != Some(absolute) {
        return Err(corrupt());
    }
    if absolute >= 20 {
        let mut locator = [0; 20];
        read_exact(source, absolute - 20, &mut locator)?;
        if locator.starts_with(b"PK\x06\x07") {
            return Err(limit());
        }
    }
    Ok(())
}
fn safe_path(path: &str) -> bool {
    !path.is_empty()
        && path.len() <= 4096
        && !path.starts_with('/')
        && !path.contains(['\\', ':', '\0'])
        && !path.chars().any(char::is_control)
        && path.split('/').all(|part| !matches!(part, "" | "." | ".."))
}
fn resolved(base: &str, href: &str) -> Result<String, PackError> {
    let mut origin = url::Url::parse("https://book.invalid/").map_err(|_| corrupt())?;
    origin.set_path(base);
    let target = origin.join(href).map_err(|_| corrupt())?;
    if target.origin() != origin.origin()
        || target.query().is_some()
        || target.fragment().is_some()
        || target.username() != ""
        || target.password().is_some()
    {
        return Err(corrupt());
    }
    let path = percent_encoding::percent_decode_str(target.path())
        .decode_utf8()
        .map_err(|_| corrupt())?;
    let path = path.strip_prefix('/').ok_or_else(corrupt)?;
    if !safe_path(path) {
        return Err(corrupt());
    }
    Ok(path.into())
}
impl Archive {
    pub(crate) fn open(source: Arc<dyn PackSource>) -> Result<Self, PackError> {
        preflight(source.as_ref())?;
        let input = Positioned {
            source,
            offset: 0,
            budget: 64 * 1024 * 1024,
        };
        let mut archive = ZipArchive::with_config(
            Config {
                archive_offset: ArchiveOffset::Known(0),
            },
            input,
        )
        .map_err(zip_error)?;
        if archive.len() > MAX_FILES || archive.has_duplicate_names() {
            return Err(corrupt());
        }
        let mut members = HashMap::new();
        let mut expanded = 0u64;
        let mut name_bytes = 0usize;
        for index in 0..archive.len() {
            let file = archive.by_index_raw(index).map_err(zip_error)?;
            let name = file.name().map_err(zip_error)?.into_owned();
            name_bytes = name_bytes.saturating_add(name.len());
            if name_bytes > 4 * 1024 * 1024 {
                return Err(limit());
            }
            if !safe_path(if file.is_dir() {
                name.trim_end_matches('/')
            } else {
                &name
            }) || file.is_symlink()
                || file.encrypted()
                || !matches!(
                    file.compression(),
                    CompressionMethod::Stored | CompressionMethod::Deflated
                )
            {
                return Err(corrupt());
            }
            expanded = expanded.checked_add(file.size()).ok_or_else(limit)?;
            if file.size() > MAX_MEMBER
                || file.compressed_size() > MAX_MEMBER
                || expanded > MAX_EXPANDED
            {
                return Err(limit());
            }
            if file.is_dir() {
                continue;
            }
            if members.insert(name, index).is_some() {
                return Err(corrupt());
            }
        }
        archive.validate_header_consistency().map_err(zip_error)?;
        Ok(Self { archive, members })
    }
    pub(crate) fn read(&self, path: &str, maximum: u64) -> Result<Vec<u8>, PackError> {
        let index = *self
            .members
            .get(path)
            .ok_or_else(|| PackError::NotFound("Archive resource".into()))?;
        let mut archive = self.archive.clone();
        let file = archive.by_index(index).map_err(zip_error)?;
        let expected = file.size();
        if expected > maximum {
            return Err(limit());
        }
        let mut bytes = Vec::with_capacity(expected as usize);
        file.take(maximum + 1)
            .read_to_end(&mut bytes)
            .map_err(|error| zip_error(ZipError::Io(error)))?;
        if bytes.len() as u64 != expected {
            return Err(corrupt());
        }
        Ok(bytes)
    }
}
impl Book {
    pub fn open(source: Arc<dyn PackSource>, kind: BookKind) -> Result<Self, PackError> {
        let mut book = Self {
            archive: Archive::open(source)?,
            metadata: BookMetadata {
                kind,
                chapters: Vec::new(),
            },
        };
        let paths = match kind {
            BookKind::Comic => {
                let mut paths: Vec<_> = book
                    .archive
                    .members
                    .keys()
                    .filter(|path| raster(path))
                    .cloned()
                    .collect();
                paths.sort_by(|a, b| natural(a, b));
                paths
            }
            BookKind::Epub => book.epub_spine()?,
        };
        if paths.is_empty() {
            return Err(corrupt());
        }
        book.metadata.chapters = paths
            .into_iter()
            .enumerate()
            .map(|(index, path)| Chapter {
                path,
                title: (index + 1).to_string(),
            })
            .collect();
        Ok(book)
    }
    pub fn metadata(&self) -> &BookMetadata {
        &self.metadata
    }
    fn read(&self, path: &str, maximum: u64) -> Result<Vec<u8>, PackError> {
        self.archive.read(path, maximum)
    }
    pub fn resource(&self, path: &str) -> Result<Resource, PackError> {
        if !safe_path(path) {
            return Err(PackError::NotFound("Book resource".into()));
        }
        let mime = match extension(path).as_str() {
            "xhtml" | "html" | "htm" => "text/html; charset=utf-8",
            "css" => "text/css; charset=utf-8",
            "png" => "image/png",
            "jpg" | "jpeg" => "image/jpeg",
            "gif" => "image/gif",
            "webp" => "image/webp",
            "avif" => "image/avif",
            "bmp" => "image/bmp",
            "svg" => "image/svg+xml",
            "woff" => "font/woff",
            "woff2" => "font/woff2",
            "ttf" => "font/ttf",
            "otf" => "font/otf",
            _ => "application/octet-stream",
        };
        Ok(Resource {
            mime,
            bytes: self.read(path, MAX_MEMBER)?,
        })
    }
    fn epub_spine(&self) -> Result<Vec<String>, PackError> {
        if self.read("mimetype", 64)? != b"application/epub+zip" {
            return Err(corrupt());
        }
        let container = elements(&self.read("META-INF/container.xml", MAX_XML)?)?;
        let opf = container
            .iter()
            .find(|element| element.name == "rootfile")
            .and_then(|element| element.attributes.get("full-path"))
            .ok_or_else(corrupt)?;
        let opf = resolved("", opf)?;
        let package = elements(&self.read(&opf, MAX_XML)?)?;
        let mut manifest = HashMap::new();
        for element in package.iter().filter(|element| element.name == "item") {
            let id = element.attributes.get("id").ok_or_else(corrupt)?;
            let href = element.attributes.get("href").ok_or_else(corrupt)?;
            let kind = element.attributes.get("media-type").ok_or_else(corrupt)?;
            let path = resolved(&opf, href)?;
            if !self.archive.members.contains_key(&path)
                || manifest.insert(id.clone(), (path, kind)).is_some()
            {
                return Err(corrupt());
            }
        }
        package
            .iter()
            .filter(|element| {
                element.name == "itemref"
                    && element.attributes.get("linear").map(String::as_str) != Some("no")
            })
            .map(|element| {
                let id = element.attributes.get("idref").ok_or_else(corrupt)?;
                let (path, kind) = manifest.get(id).ok_or_else(corrupt)?;
                let supported = match kind.as_str() {
                    "application/xhtml+xml" | "text/html" => {
                        matches!(extension(path).as_str(), "xhtml" | "html" | "htm")
                    }
                    "image/svg+xml" => extension(path) == "svg",
                    _ => false,
                };
                if !supported {
                    return Err(corrupt());
                }
                Ok(path.clone())
            })
            .collect()
    }
}
struct Element {
    name: String,
    attributes: HashMap<String, String>,
}
fn elements(bytes: &[u8]) -> Result<Vec<Element>, PackError> {
    let mut reader = Reader::from_reader(DecodingReader::new(bytes));
    reader.config_mut().trim_text(true);
    let mut buffer = Vec::new();
    let mut output = Vec::new();
    let mut depth = 0usize;
    let mut roots = 0;
    let mut version = XmlVersion::Implicit1_0;
    loop {
        let event = reader.read_event_into(&mut buffer).map_err(|_| corrupt())?;
        let empty = matches!(&event, Event::Empty(_));
        match event {
            Event::Decl(declaration) => {
                version = declaration.xml_version().map_err(|_| corrupt())?;
                if declaration.encoding().is_some() {
                    let encoding = declaration.encoder().ok_or_else(corrupt)?;
                    reader.get_mut().set_encoding(encoding);
                }
            }
            Event::DocType(value) => {
                // External declarations are inert; internal entity definitions are refused.
                if value.as_ref().contains(['[', ']']) {
                    return Err(corrupt());
                }
            }
            Event::Start(ref element) | Event::Empty(ref element) => {
                let mut attributes = HashMap::new();
                for attribute in element.attributes() {
                    let attribute = attribute.map_err(|_| corrupt())?;
                    let name = attribute.key.local_name().as_ref().to_string();
                    let value = attribute
                        .normalized_value(version)
                        .map_err(|_| corrupt())?
                        .into_owned();
                    if attributes.insert(name, value).is_some() {
                        return Err(corrupt());
                    }
                }
                let name = element.local_name().as_ref().to_string();
                output.push(Element { name, attributes });
                if depth == 0 {
                    roots += 1;
                    if roots > 1 {
                        return Err(corrupt());
                    }
                }
                if !empty {
                    depth += 1;
                    if depth > 256 {
                        return Err(limit());
                    }
                }
                if output.len() > MAX_FILES * 4 {
                    return Err(limit());
                }
            }
            Event::End(_) => {
                depth = depth.checked_sub(1).ok_or_else(corrupt)?;
            }
            Event::Eof => {
                if depth != 0 || roots != 1 {
                    return Err(corrupt());
                }
                return Ok(output);
            }
            _ => {}
        }
        buffer.clear();
    }
}
fn extension(path: &str) -> String {
    path.rsplit('.').next().unwrap_or("").to_ascii_lowercase()
}
fn raster(path: &str) -> bool {
    matches!(
        extension(path).as_str(),
        "jpg" | "jpeg" | "png" | "gif" | "webp" | "avif" | "bmp"
    )
}
fn natural(left: &str, right: &str) -> Ordering {
    let (a, b) = (left.as_bytes(), right.as_bytes());
    let (mut i, mut j) = (0, 0);
    while i < a.len() && j < b.len() {
        if a[i].is_ascii_digit() && b[j].is_ascii_digit() {
            let (mut x, mut y) = (i, j);
            while x < a.len() && a[x].is_ascii_digit() {
                x += 1;
            }
            while y < b.len() && b[y].is_ascii_digit() {
                y += 1;
            }
            let (mut first, mut second) = (i, j);
            while first + 1 < x && a[first] == b'0' {
                first += 1;
            }
            while second + 1 < y && b[second] == b'0' {
                second += 1;
            }
            let order = (x - first)
                .cmp(&(y - second))
                .then_with(|| a[first..x].cmp(&b[second..y]));
            if order != Ordering::Equal {
                return order;
            }
            i = x;
            j = y;
        } else {
            let order = a[i].to_ascii_lowercase().cmp(&b[j].to_ascii_lowercase());
            if order != Ordering::Equal {
                return order;
            }
            i += 1;
            j += 1;
        }
    }
    a.len().cmp(&b.len()).then_with(|| left.cmp(right))
}
