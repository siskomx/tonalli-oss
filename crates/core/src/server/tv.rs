//! Selected-media TV catalog. Network lifecycle and native opt-in are wired separately.
mod description;
mod events;
mod http;
mod soap;
mod ssdp;
mod transport;
use super::SharedMedia;
use crate::pack::{PackError, PackSource};
pub use http::TvServer;
use std::{net::SocketAddr, sync::Arc};

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum TvError {
    #[error("Invalid TV browse request")]
    Invalid,
    #[error("This TV item is unavailable")]
    Missing,
    #[error("This TV item is not a folder")]
    NotContainer,
    #[error("This TV sort order is unsupported")]
    SortUnsupported,
    #[error("This TV action is unsupported")]
    Action,
    #[error("This TV connection is unavailable")]
    ConnectionMissing,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TvService {
    ContentDirectory,
    ConnectionManager,
}
fn random_uuid() -> Result<String, PackError> {
    let mut bytes = [0; 16];
    getrandom::fill(&mut bytes).map_err(|problem| PackError::Io(problem.to_string()))?;
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let hex = bytes
        .iter()
        .map(|value| format!("{value:02x}"))
        .collect::<String>();
    Ok(format!(
        "{}-{}-{}-{}-{}",
        &hex[..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..]
    ))
}
pub struct TvBrowse<'a> {
    pub object_id: &'a str,
    pub metadata: bool,
    pub start: u32,
    pub count: u32,
    pub sort: &'a str,
}
pub struct TvPage {
    pub xml: String,
    pub returned: u32,
    pub total: u32,
}
struct Entry {
    file: crate::media::MediaFile,
    source: Arc<dyn PackSource>,
    mime: &'static str,
    cover: Option<String>,
}
#[cfg_attr(feature = "ffi", derive(uniffi::Object))]
pub struct TvCatalog {
    entries: Vec<Entry>,
}
#[cfg_attr(feature = "ffi", uniffi::export)]
impl TvCatalog {
    pub fn selected_media(&self) -> Vec<crate::media::MediaFile> {
        self.entries
            .iter()
            .map(|entry| entry.file.clone())
            .collect()
    }
}
impl TvCatalog {
    pub fn new(media: Arc<SharedMedia>) -> Result<Self, PackError> {
        if media.files.is_empty() {
            return Err(PackError::Mismatch("Choose media for TV mode".into()));
        }
        let mut entries = Vec::with_capacity(media.files.len());
        for entry in &media.files {
            let mime = entry
                .mime
                .filter(|mime| {
                    mime.starts_with("audio/")
                        || mime.starts_with("video/")
                        || mime.starts_with("image/")
                })
                .ok_or_else(|| {
                    PackError::Mismatch(
                        "TV mode supports validated audio, video and images only".into(),
                    )
                })?;
            entries.push(Entry {
                file: entry.file.clone(),
                source: Arc::clone(&entry.source),
                mime,
                cover: None,
            });
        }
        // Match the existing video catalog convention, strictly inside the explicit selection.
        for index in 0..entries.len() {
            if entries[index].mime.starts_with("image/") {
                continue;
            }
            let file = &entries[index].file;
            let mut cover = None;
            for wanted in [stem(&file.name), "poster", "cover", "folder"] {
                if let Some(image) = entries.iter().find(|image| {
                    matches!(image.mime, "image/jpeg" | "image/png" | "image/webp")
                        && image.file.folder == file.folder
                        && stem(&image.file.name).to_lowercase() == wanted.to_lowercase()
                }) {
                    cover = Some(image.file.id.clone());
                    break;
                }
            }
            entries[index].cover = cover;
        }
        Ok(Self { entries })
    }
    pub fn browse(&self, address: SocketAddr, request: TvBrowse<'_>) -> Result<TvPage, TvError> {
        self.browse_filtered(address, request, "*")
    }
    fn browse_filtered(
        &self,
        address: SocketAddr,
        request: TvBrowse<'_>,
        filter: &str,
    ) -> Result<TvPage, TvError> {
        if address.port() == 0 || super::lan::validate_bind(address.ip()).is_err() {
            return Err(TvError::Invalid);
        }
        if !matches!(request.sort, "" | "+dc:title" | "-dc:title") {
            return Err(TvError::SortUnsupported);
        }
        let filter = Filter::new(filter)?;
        let mut xml=String::from("<DIDL-Lite xmlns=\"urn:schemas-upnp-org:metadata-1-0/DIDL-Lite/\" xmlns:dc=\"http://purl.org/dc/elements/1.1/\" xmlns:upnp=\"urn:schemas-upnp-org:metadata-1-0/upnp/\">");
        let (returned, total) = if request.object_id == "0" {
            if request.metadata {
                let count = if filter.child_count {
                    format!(" childCount=\"{}\"", self.entries.len())
                } else {
                    String::new()
                };
                xml.push_str(&format!("<container id=\"0\" parentID=\"-1\" restricted=\"1\"{count}><dc:title>Tonalli</dc:title><upnp:class>object.container</upnp:class></container>"));
                (1, 1)
            } else {
                let mut entries = self.entries.iter().collect::<Vec<_>>();
                if !request.sort.is_empty() {
                    entries.sort_by(|a, b| {
                        a.file
                            .name
                            .to_lowercase()
                            .cmp(&b.file.name.to_lowercase())
                            .then_with(|| a.file.id.cmp(&b.file.id))
                    });
                    if request.sort.starts_with('-') {
                        entries.reverse();
                    }
                }
                let count = if request.count == 0 {
                    entries.len()
                } else {
                    request.count as usize
                };
                let total = entries.len() as u32;
                let mut returned = 0;
                for entry in entries.into_iter().skip(request.start as usize).take(count) {
                    render(entry, address, &filter, &mut xml);
                    returned += 1;
                }
                (returned, total)
            }
        } else {
            let entry = self
                .entries
                .iter()
                .find(|entry| entry.file.id == request.object_id)
                .ok_or(TvError::Missing)?;
            if !request.metadata {
                return Err(TvError::NotContainer);
            }
            render(entry, address, &filter, &mut xml);
            (1, 1)
        };
        xml.push_str("</DIDL-Lite>");
        Ok(TvPage {
            xml,
            returned,
            total,
        })
    }
    pub fn source(&self, id: &str) -> Option<(Arc<dyn PackSource>, &'static str)> {
        self.entries
            .iter()
            .find(|entry| entry.file.id == id)
            .map(|entry| (Arc::clone(&entry.source), entry.mime))
    }
    pub fn protocol_info(&self) -> String {
        self.entries
            .iter()
            .map(|entry| format!("http-get:*:{}:*", entry.mime))
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>()
            .join(",")
    }
}
fn stem(name: &str) -> &str {
    name.rsplit_once('.').map_or(name, |(stem, _)| stem)
}
fn escaped(value: &str) -> String {
    let clean = value
        .chars()
        .map(|value| {
            if xml_character(value) {
                value
            } else {
                '\u{fffd}'
            }
        })
        .collect::<String>();
    quick_xml::escape::escape(clean).into_owned()
}
fn xml_character(value: char) -> bool {
    matches!(value,'\t'|'\n'|'\r'|'\u{20}'..='\u{d7ff}'|'\u{e000}'..='\u{fffd}'|'\u{10000}'..='\u{10ffff}')
}
struct Filter {
    resource: bool,
    size: bool,
    cover: bool,
    child_count: bool,
}
impl Filter {
    fn new(value: &str) -> Result<Self, TvError> {
        if value.len() > 512 {
            return Err(TvError::Invalid);
        }
        let parts = value.split(',').map(str::trim).collect::<Vec<_>>();
        if parts.len() > 64 {
            return Err(TvError::Invalid);
        }
        let all = parts.contains(&"*");
        let resource = all
            || parts
                .iter()
                .any(|value| *value == "res" || value.starts_with("res@"));
        Ok(Self {
            resource,
            size: all
                || parts.contains(&"res")
                || parts.contains(&"res@size")
                || parts.contains(&"res@*"),
            cover: all || parts.contains(&"upnp:albumArtURI") || parts.contains(&"upnp:*"),
            child_count: all
                || parts.contains(&"@childCount")
                || parts.contains(&"container@childCount"),
        })
    }
}
fn render(entry: &Entry, address: SocketAddr, filter: &Filter, xml: &mut String) {
    let class = if entry.mime.starts_with("audio/") {
        "object.item.audioItem.musicTrack"
    } else if entry.mime.starts_with("video/") {
        "object.item.videoItem"
    } else {
        "object.item.imageItem.photo"
    };
    xml.push_str(&format!("<item id=\"{}\" parentID=\"0\" restricted=\"1\"><dc:title>{}</dc:title><upnp:class>{class}</upnp:class>",escaped(&entry.file.id),escaped(&entry.file.name)));
    if filter.resource {
        let size = if filter.size {
            format!(" size=\"{}\"", entry.file.bytes)
        } else {
            String::new()
        };
        xml.push_str(&format!(
            "<res protocolInfo=\"http-get:*:{}:*\"{size}>http://{address}/media/{}</res>",
            entry.mime,
            escaped(&entry.file.id)
        ));
    }
    if let Some(cover) = entry.cover.as_ref().filter(|_| filter.cover) {
        xml.push_str(&format!(
            "<upnp:albumArtURI>http://{address}/media/{}</upnp:albumArtURI>",
            escaped(cover)
        ));
    }
    xml.push_str("</item>");
}
