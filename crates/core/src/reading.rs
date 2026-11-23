//! Private reading history/bookmarks. Never mounted on a LAN content router.
use crate::{content::storage, download::write_atomic, pack::PackError, server::PackId};
use serde::{Deserialize, Serialize};
use std::{
    fs::File,
    io::Read,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Destination {
    Article { pack: String, url: String },
    Media { id: String },
    Map { pack: String },
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Position {
    Start,
    Scroll { y: f64 },
    Page { number: u32 },
    Time { seconds: f64 },
    Epub { location: String },
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReadingEntry {
    pub destination: Destination,
    pub position: Position,
    pub title: String,
    pub bookmarked: bool,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Document {
    version: u32,
    entries: Vec<ReadingEntry>,
}
pub struct ProgressStore {
    root: PathBuf,
}
fn invalid() -> PackError {
    PackError::Corrupt("Invalid reading progress metadata".into())
}
fn text(value: &str, max: usize) -> bool {
    !value.is_empty() && value.len() <= max && !value.chars().any(char::is_control)
}
impl Destination {
    pub fn valid(&self) -> bool {
        match self {
            Self::Article { pack, url } => {
                let decoded = percent_encoding::percent_decode_str(url).decode_utf8();
                PackId::new(pack.clone()).is_ok()
                    && text(url, 4096)
                    && decoded.is_ok_and(|path| {
                        text(&path, 4096)
                            && !path.starts_with('/')
                            && !path.contains('\\')
                            && !path.split('/').next().unwrap_or("").contains(':')
                            && !path.split('/').any(|part| part == "." || part == "..")
                    })
            }
            Self::Map { pack } => PackId::new(pack.clone()).is_ok(),
            Self::Media { id } => {
                id.len() == 32
                    && id
                        .bytes()
                        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            }
        }
    }
}
impl ReadingEntry {
    fn valid(&self) -> bool {
        self.destination.valid()
            && text(&self.title, 1024)
            && match (&self.destination, &self.position) {
                (_, Position::Start) => true,
                (
                    Destination::Article { .. } | Destination::Media { .. },
                    Position::Scroll { y },
                ) => y.is_finite() && (0.0..=1_000_000_000.0).contains(y),
                (
                    Destination::Article { .. } | Destination::Media { .. },
                    Position::Page { number },
                ) => (1..=1_000_000).contains(number),
                (Destination::Media { .. }, Position::Time { seconds }) => {
                    seconds.is_finite() && (0.0..=1_000_000_000.0).contains(seconds)
                }
                (
                    Destination::Article { .. } | Destination::Media { .. },
                    Position::Epub { location },
                ) => text(location, 4096),
                _ => false,
            }
    }
}
impl ProgressStore {
    pub fn open(root: &Path) -> Result<Self, PackError> {
        Ok(Self {
            root: root.canonicalize()?,
        })
    }
    fn path(&self, name: &str) -> Result<PathBuf, PackError> {
        storage::path(&self.root, &format!("packs/private/{name}"))
    }
    fn read(&self) -> Result<Document, PackError> {
        let file = match File::open(self.path("reading.json")?) {
            Ok(file) => file,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Document {
                    version: 1,
                    entries: vec![],
                })
            }
            Err(e) => return Err(e.into()),
        };
        let mut bytes = Vec::new();
        file.take(1024 * 1024 + 1).read_to_end(&mut bytes)?;
        if bytes.len() > 1024 * 1024 {
            return Err(invalid());
        }
        let document: Document = serde_json::from_slice(&bytes).map_err(|_| invalid())?;
        if document.version != 1
            || document.entries.len() > 128
            || document.entries.iter().any(|entry| !entry.valid())
        {
            return Err(invalid());
        }
        for (index, entry) in document.entries.iter().enumerate() {
            if document.entries[..index]
                .iter()
                .any(|other| other.destination == entry.destination)
            {
                return Err(invalid());
            }
        }
        Ok(document)
    }
    fn write(&self, document: &Document) -> Result<(), PackError> {
        let bytes = serde_json::to_vec(document).map_err(|_| invalid())?;
        if bytes.len() > 1024 * 1024 {
            return Err(invalid());
        }
        let path = self.path("reading.json")?;
        write_atomic(&path, &bytes)?;
        storage::durable(&path)
    }
    pub fn list(&self) -> Result<Vec<ReadingEntry>, PackError> {
        let _lock = storage::lock(&self.path("reading.lock")?)?;
        Ok(self.read()?.entries)
    }
    pub fn save(&self, mut entry: ReadingEntry) -> Result<(), PackError> {
        if !entry.valid() {
            return Err(invalid());
        }
        let _lock = storage::lock(&self.path("reading.lock")?)?;
        let mut document = self.read()?;
        if let Some(index) = document
            .entries
            .iter()
            .position(|old| old.destination == entry.destination)
        {
            entry.bookmarked |= document.entries[index].bookmarked;
            document.entries.remove(index);
        }
        if document.entries.len() == 128 {
            let index = document
                .entries
                .iter()
                .rposition(|old| !old.bookmarked)
                .ok_or(PackError::NoSpace)?;
            document.entries.remove(index);
        }
        document.entries.insert(0, entry);
        self.write(&document)
    }
    pub fn bookmark(&self, destination: &Destination, value: bool) -> Result<(), PackError> {
        if !destination.valid() {
            return Err(invalid());
        }
        let _lock = storage::lock(&self.path("reading.lock")?)?;
        let mut document = self.read()?;
        let entry = document
            .entries
            .iter_mut()
            .find(|entry| &entry.destination == destination)
            .ok_or_else(invalid)?;
        entry.bookmarked = value;
        self.write(&document)
    }
    pub fn remove(&self, destination: &Destination) -> Result<(), PackError> {
        if !destination.valid() {
            return Err(invalid());
        }
        let _lock = storage::lock(&self.path("reading.lock")?)?;
        let mut document = self.read()?;
        document
            .entries
            .retain(|entry| &entry.destination != destination);
        self.write(&document)
    }
}
