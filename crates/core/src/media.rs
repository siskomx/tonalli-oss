//! App-managed media copies. This catalog is separate from signed packs and encrypted Notes.
mod native_import;
use crate::{
    content::storage,
    download::check_space,
    pack::{verify_sha256, PackError, PackSource, PreadSource},
};
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::Arc,
};

pub const MAX_FILE_BYTES: u64 = 64 * 1024 * 1024 * 1024;
pub const MAX_TEXT_BYTES: u64 = 65536;
#[derive(Debug, thiserror::Error)]
pub enum MediaError {
    #[error("Import stopped before publication")]
    Cancelled,
    #[error("Invalid file name, folder or text input")]
    Invalid,
    #[error("This file changed; reload before trying again")]
    Conflict,
    #[error("File is no longer in the managed library")]
    NotFound,
    #[error("Media catalog is unavailable or corrupt")]
    Catalog,
    #[error(transparent)]
    Pack(#[from] PackError),
}
impl From<std::io::Error> for MediaError {
    fn from(error: std::io::Error) -> Self {
        Self::Pack(error.into())
    }
}
impl From<rusqlite::Error> for MediaError {
    fn from(_: rusqlite::Error) -> Self {
        Self::Catalog
    }
}
#[derive(Clone, Debug, Serialize)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct MediaFile {
    pub id: String,
    pub name: String,
    pub folder: String,
    pub version: u64,
    pub bytes: u64,
    pub sha256: String,
}
impl MediaFile {
    pub fn editable(&self) -> bool {
        self.bytes <= MAX_TEXT_BYTES
            && matches!(
                self.name
                    .rsplit('.')
                    .next()
                    .unwrap_or("")
                    .to_ascii_lowercase()
                    .as_str(),
                "txt" | "md" | "csv" | "json" | "toml" | "yaml" | "yml" | "log" | "cook"
            )
    }
}
#[derive(Clone)]
pub struct MediaStore {
    root: PathBuf,
}
pub struct Upload {
    store: MediaStore,
    temporary: tempfile::NamedTempFile,
    name: String,
    folder: String,
}
impl Upload {
    pub fn file(&self) -> Result<File, MediaError> {
        Ok(self.temporary.reopen()?)
    }
    pub fn finish(self) -> Result<MediaFile, MediaError> {
        self.store.clone().publish(self, None)
    }
}
impl MediaStore {
    pub fn open(root: &Path) -> Result<Self, MediaError> {
        let store = Self {
            root: root.canonicalize()?,
        };
        for name in ["staging/guard", "blobs/guard"] {
            let path = store.path(name)?;
            fs::create_dir_all(path.parent().ok_or(MediaError::Catalog)?)?;
        }
        let (_lock, _db) = store.database()?;
        Ok(store)
    }
    fn path(&self, name: &str) -> Result<PathBuf, MediaError> {
        Ok(storage::path(&self.root, &format!("packs/media/{name}"))?)
    }
    fn database(&self) -> Result<(File, Connection), MediaError> {
        let lock = storage::lock(&self.path("catalog.lock")?)?;
        for name in [
            "catalog.sqlite-journal",
            "catalog.sqlite-wal",
            "catalog.sqlite-shm",
        ] {
            self.path(name)?;
        }
        let mut db = Connection::open(self.path("catalog.sqlite")?)?;
        db.execute_batch("PRAGMA mmap_size=0; PRAGMA trusted_schema=OFF; PRAGMA synchronous=FULL; PRAGMA journal_mode=DELETE;")?;
        let version: u32 = db.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        if version == 0 {
            let tables: u32 = db.query_row(
                "SELECT count(*) FROM sqlite_master WHERE type='table'",
                [],
                |r| r.get(0),
            )?;
            if tables != 0 {
                return Err(MediaError::Catalog);
            }
            let tx = db.transaction()?;
            tx.execute_batch("CREATE TABLE files(id TEXT PRIMARY KEY, name TEXT NOT NULL, folder TEXT NOT NULL, version INTEGER NOT NULL, bytes INTEGER NOT NULL, sha256 TEXT NOT NULL); PRAGMA user_version=1;")?;
            tx.commit()?;
        } else if version != 1 {
            return Err(MediaError::Catalog);
        }
        Ok((lock, db))
    }
    pub fn begin(&self, name: &str, folder: &str) -> Result<Upload, MediaError> {
        if !valid_name(name) || !valid_folder(folder) {
            return Err(MediaError::Invalid);
        }
        let directory = self.path("staging/guard")?;
        let temporary =
            tempfile::NamedTempFile::new_in(directory.parent().ok_or(MediaError::Catalog)?)?;
        Ok(Upload {
            store: self.clone(),
            temporary,
            name: name.into(),
            folder: folder.into(),
        })
    }
    pub fn import(
        &self,
        source: Arc<dyn PackSource>,
        name: &str,
        folder: &str,
    ) -> Result<MediaFile, MediaError> {
        self.import_controlled(source, name, folder, || true)
    }
    pub fn import_controlled(
        &self,
        source: Arc<dyn PackSource>,
        name: &str,
        folder: &str,
        mut keep_going: impl FnMut() -> bool,
    ) -> Result<MediaFile, MediaError> {
        if !keep_going() {
            return Err(MediaError::Cancelled);
        }
        let mut upload = self.begin(name, folder)?;
        if source.len() > MAX_FILE_BYTES {
            return Err(MediaError::Invalid);
        }
        check_space(fs2::available_space(&self.root)?, source.len())?;
        let mut offset = 0;
        let mut buffer = [0; 65536];
        while offset < source.len() {
            if !keep_going() {
                return Err(MediaError::Cancelled);
            }
            let count = (source.len() - offset).min(buffer.len() as u64) as usize;
            let read = source.read_at(offset, &mut buffer[..count])?;
            if read == 0 {
                return Err(PackError::Truncated.into());
            }
            if read > count {
                return Err(PackError::Corrupt("Invalid media read length".into()).into());
            }
            upload.temporary.write_all(&buffer[..read])?;
            offset += read as u64;
        }
        self.publish_checked(upload, None, &mut keep_going)
    }
    fn publish(
        &self,
        upload: Upload,
        previous: Option<MediaFile>,
    ) -> Result<MediaFile, MediaError> {
        self.publish_checked(upload, previous, &mut || true)
    }
    fn publish_checked(
        &self,
        upload: Upload,
        previous: Option<MediaFile>,
        keep_going: &mut impl FnMut() -> bool,
    ) -> Result<MediaFile, MediaError> {
        upload.temporary.as_file().sync_all()?;
        let bytes = upload.temporary.as_file().metadata()?.len();
        if bytes > MAX_FILE_BYTES {
            return Err(MediaError::Invalid);
        }
        let mut source = upload.temporary.reopen()?;
        let mut hasher = Sha256::new();
        let mut buffer = [0; 65536];
        loop {
            if !keep_going() {
                return Err(MediaError::Cancelled);
            }
            let n = source.read(&mut buffer)?;
            if n == 0 {
                break;
            }
            hasher.update(&buffer[..n]);
        }
        let sha256 = format!("{:x}", hasher.finalize());
        verify_copy(upload.temporary.path(), &sha256, keep_going)?;
        let (_lock, mut db) = self.database()?;
        let id = if let Some(ref old) = previous {
            let current = find(&db, &old.id)?;
            if current.version != old.version {
                return Err(MediaError::Conflict);
            }
            old.id.clone()
        } else {
            let count: i64 = db.query_row("SELECT count(*) FROM files", [], |r| r.get(0))?;
            if count >= 10000 {
                return Err(MediaError::Invalid);
            }
            let mut random = [0; 16];
            getrandom::fill(&mut random).map_err(|e| PackError::Io(e.to_string()))?;
            random.iter().map(|b| format!("{b:02x}")).collect()
        };
        let version = previous
            .as_ref()
            .map_or(Some(1), |old| old.version.checked_add(1))
            .filter(|v| *v <= i64::MAX as u64)
            .ok_or(MediaError::Conflict)?;
        let path = self.blob(&sha256)?;
        if path.exists() {
            verify_copy(&path, &sha256, keep_going)?;
        } else {
            if !keep_going() {
                return Err(MediaError::Cancelled);
            }
            upload
                .temporary
                .persist_noclobber(&path)
                .map_err(|e| MediaError::from(e.error))?;
        }
        storage::durable(&path)?;
        let row = MediaFile {
            id,
            name: upload.name,
            folder: upload.folder,
            version,
            bytes,
            sha256,
        };
        let tx = db.transaction()?;
        if previous.is_some() {
            tx.execute(
                "UPDATE files SET name=?2,folder=?3,version=?4,bytes=?5,sha256=?6 WHERE id=?1",
                params![
                    row.id,
                    row.name,
                    row.folder,
                    (row.version as i64),
                    (row.bytes as i64),
                    row.sha256
                ],
            )?;
        } else {
            tx.execute(
                "INSERT INTO files VALUES(?1,?2,?3,?4,?5,?6)",
                params![
                    row.id,
                    row.name,
                    row.folder,
                    (row.version as i64),
                    (row.bytes as i64),
                    row.sha256
                ],
            )?;
        }
        tx.commit()
            .map_err(|error| PackError::PublicationUncertain(error.to_string()))?;
        if let Some(old) = previous {
            self.remove_unused(&db, &old.sha256)?;
        }
        Ok(row)
    }
    fn blob(&self, sha: &str) -> Result<PathBuf, MediaError> {
        if !hex(sha, 64) {
            return Err(MediaError::Catalog);
        }
        self.path(&format!("blobs/{sha}.blob"))
    }
    fn remove_unused(&self, db: &Connection, sha: &str) -> Result<(), MediaError> {
        let count: i64 =
            db.query_row("SELECT count(*) FROM files WHERE sha256=?1", [sha], |r| {
                r.get(0)
            })?;
        if count == 0 {
            match fs::remove_file(self.blob(sha)?) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => {
                    return Err(PackError::PublicationUncertain(format!(
                        "Library updated; old managed copy could not be removed: {e}"
                    ))
                    .into())
                }
            }
        }
        Ok(())
    }
    pub fn list(&self) -> Result<Vec<MediaFile>, MediaError> {
        let (_lock, db) = self.database()?;
        let mut statement=db.prepare("SELECT id,name,folder,version,bytes,sha256 FROM files ORDER BY folder,name,id LIMIT 10001")?;
        let rows = statement
            .query_map([], decode)?
            .collect::<Result<Vec<_>, _>>()?;
        if rows.len() > 10000 {
            return Err(MediaError::Catalog);
        }
        for row in &rows {
            validate(row)?;
        }
        Ok(rows)
    }
    pub fn get(&self, id: &str) -> Result<MediaFile, MediaError> {
        let (_lock, db) = self.database()?;
        find(&db, id)
    }
    pub fn source(&self, id: &str) -> Result<Arc<dyn PackSource>, MediaError> {
        self.open_file(id).map(|(_, source)| source)
    }
    pub fn open_file(&self, id: &str) -> Result<(MediaFile, Arc<dyn PackSource>), MediaError> {
        let (_lock, db) = self.database()?;
        let row = find(&db, id)?;
        let source = PreadSource::open(&self.blob(&row.sha256)?)?;
        if source.len() != row.bytes {
            return Err(PackError::Truncated.into());
        }
        Ok((row, Arc::new(source)))
    }
    pub fn export_to(&self, id: &str, output: &mut impl Write) -> Result<(), MediaError> {
        let (row, source) = self.open_file(id)?;
        let mut offset = 0;
        let mut buffer = [0; 65536];
        let mut digest = Sha256::new();
        while offset < source.len() {
            let count = (source.len() - offset).min(buffer.len() as u64) as usize;
            let n = source.read_at(offset, &mut buffer[..count])?;
            if n == 0 {
                return Err(PackError::Truncated.into());
            }
            if n > count {
                return Err(PackError::Corrupt("Invalid media read length".into()).into());
            }
            digest.update(&buffer[..n]);
            output.write_all(&buffer[..n])?;
            offset += n as u64;
        }
        if format!("{:x}", digest.finalize()) != row.sha256 {
            return Err(PackError::Checksum.into());
        }
        Ok(())
    }
    pub fn read_text(&self, id: &str) -> Result<String, MediaError> {
        let (_lock, db) = self.database()?;
        let row = find(&db, id)?;
        if !row.editable() {
            return Err(MediaError::Invalid);
        }
        let path = self.blob(&row.sha256)?;
        verify_sha256(&path, &row.sha256)?;
        let mut bytes = Vec::new();
        File::open(path)?
            .take(MAX_TEXT_BYTES + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() as u64 > MAX_TEXT_BYTES || bytes.contains(&0) {
            return Err(MediaError::Invalid);
        }
        String::from_utf8(bytes).map_err(|_| MediaError::Invalid)
    }
    pub fn rename(&self, id: &str, version: u64, name: &str) -> Result<MediaFile, MediaError> {
        if !valid_name(name) {
            return Err(MediaError::Invalid);
        }
        let (_lock, db) = self.database()?;
        let mut row = find(&db, id)?;
        if row.version != version {
            return Err(MediaError::Conflict);
        }
        row.version = row
            .version
            .checked_add(1)
            .filter(|v| *v <= i64::MAX as u64)
            .ok_or(MediaError::Conflict)?;
        row.name = name.into();
        db.execute(
            "UPDATE files SET name=?2,version=?3 WHERE id=?1",
            params![row.id, row.name, (row.version as i64)],
        )?;
        Ok(row)
    }
    pub fn edit_text(&self, id: &str, version: u64, text: &str) -> Result<MediaFile, MediaError> {
        if text.len() as u64 > MAX_TEXT_BYTES || text.contains('\0') {
            return Err(MediaError::Invalid);
        }
        let old = self.get(id)?;
        if old.version != version {
            return Err(MediaError::Conflict);
        }
        self.read_text(id)?;
        let mut upload = self.begin(&old.name, &old.folder)?;
        upload.temporary.write_all(text.as_bytes())?;
        self.publish(upload, Some(old))
    }
    pub fn remove(&self, id: &str, version: u64) -> Result<(), MediaError> {
        let (_lock, mut db) = self.database()?;
        let row = find(&db, id)?;
        if row.version != version {
            return Err(MediaError::Conflict);
        }
        let tx = db.transaction()?;
        tx.execute("DELETE FROM files WHERE id=?1", [id])?;
        tx.commit()
            .map_err(|error| PackError::PublicationUncertain(error.to_string()))?;
        self.remove_unused(&db, &row.sha256)
    }
}
fn decode(row: &rusqlite::Row<'_>) -> rusqlite::Result<MediaFile> {
    Ok(MediaFile {
        id: row.get(0)?,
        name: row.get(1)?,
        folder: row.get(2)?,
        version: u64::try_from(row.get::<_, i64>(3)?).map_err(|_| rusqlite::Error::InvalidQuery)?,
        bytes: u64::try_from(row.get::<_, i64>(4)?).map_err(|_| rusqlite::Error::InvalidQuery)?,
        sha256: row.get(5)?,
    })
}
fn find(db: &Connection, id: &str) -> Result<MediaFile, MediaError> {
    if !hex(id, 32) {
        return Err(MediaError::NotFound);
    }
    let row = db
        .query_row(
            "SELECT id,name,folder,version,bytes,sha256 FROM files WHERE id=?1",
            [id],
            decode,
        )
        .optional()?
        .ok_or(MediaError::NotFound)?;
    validate(&row)?;
    Ok(row)
}
fn validate(row: &MediaFile) -> Result<(), MediaError> {
    if !hex(&row.id, 32)
        || !hex(&row.sha256, 64)
        || !valid_name(&row.name)
        || !valid_folder(&row.folder)
        || row.version == 0
        || row.bytes > MAX_FILE_BYTES
    {
        return Err(MediaError::Catalog);
    }
    Ok(())
}
fn hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn valid_name(name: &str) -> bool {
    if name.is_empty()
        || name.len() > 240
        || name.trim() != name
        || name.ends_with('.')
        || name == "."
        || name == ".."
        || name
            .chars()
            .any(|c| c.is_control() || "/\\<>:\"|?*".contains(c))
    {
        return false;
    }
    let stem = name.split('.').next().unwrap_or("").to_ascii_uppercase();
    !matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        && !(stem.len() == 4
            && (stem.starts_with("COM") || stem.starts_with("LPT"))
            && matches!(stem.as_bytes()[3], b'1'..=b'9'))
}
fn valid_folder(folder: &str) -> bool {
    folder.is_empty()
        || (folder.len() <= 1024
            && folder.split('/').count() <= 16
            && folder.split('/').all(valid_name))
}

fn verify_copy(
    path: &Path,
    hash: &str,
    keep_going: &mut impl FnMut() -> bool,
) -> Result<(), MediaError> {
    use std::ops::ControlFlow;
    match crate::pack::verify_sha256_controlled(path, hash, |_| {
        Ok(if keep_going() {
            ControlFlow::Continue(())
        } else {
            ControlFlow::Break(())
        })
    })? {
        ControlFlow::Continue(()) => Ok(()),
        ControlFlow::Break(()) => Err(MediaError::Cancelled),
    }
}
