//! SQLCipher notes with a separately wrapped, recoverable random data key.
mod envelope;
mod storage;

use std::fmt;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::time::Duration;

use rusqlite::{
    params, Connection, OpenFlags, OptionalExtension, Transaction, TransactionBehavior,
};
use subtle::ConstantTimeEq;
use thiserror::Error;
use zeroize::Zeroizing;

const SCHEMA: &str =
    "CREATE TABLE notes (id INTEGER PRIMARY KEY, title TEXT NOT NULL, body TEXT NOT NULL)";

#[derive(Debug, Error, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Error))]
pub enum NotesError {
    #[error("notes unlock failed")]
    UnlockFailed,
    #[error("notes key sidecar is missing; use the exported recovery key")]
    MissingKeys,
    #[error("note not found: {0}")]
    NotFound(i64),
    #[error("invalid notes data: {0}")]
    Corrupt(String),
    #[error("unsupported notes format: {0}")]
    Unsupported(String),
    #[error("notes destination already exists: {0}")]
    AlreadyExists(String),
    #[error("incomplete notes creation: keys at {keys}, encrypted database retained at {staged_database}; destination {database}: {reason}")]
    IncompleteCreate {
        database: String,
        keys: String,
        staged_database: String,
        reason: String,
    },
    #[error("new key envelope was published but durability/lock completion failed; try the new passphrase or recovery key: {0}")]
    RotationUncertain(String),
    #[error("notes I/O error: {0}")]
    Io(String),
    #[error("notes database error: {0}")]
    Database(String),
}

impl From<std::io::Error> for NotesError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error.to_string())
    }
}
impl From<rusqlite::Error> for NotesError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Database(error.to_string())
    }
}

/// Display explicitly exports the secret. Debug formatting is intentionally absent.
pub struct RecoveryKey(Zeroizing<[u8; 32]>);

impl fmt::Display for RecoveryKey {
    fn fmt(&self, output: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in self.0.iter() {
            write!(output, "{byte:02x}")?;
        }
        Ok(())
    }
}
impl FromStr for RecoveryKey {
    type Err = NotesError;
    fn from_str(text: &str) -> Result<Self, Self::Err> {
        if text.len() != 64 || !text.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(NotesError::Corrupt(
                "recovery key needs exactly 64 hexadecimal digits".into(),
            ));
        }
        let mut key = Zeroizing::new([0; 32]);
        for (index, pair) in text.as_bytes().chunks_exact(2).enumerate() {
            let digit = |byte: u8| {
                if byte.is_ascii_digit() {
                    byte - b'0'
                } else {
                    byte.to_ascii_lowercase() - b'a' + 10
                }
            };
            key[index] = digit(pair[0]) * 16 + digit(pair[1]);
        }
        Ok(Self(key))
    }
}

pub enum Unlock<'a> {
    Passphrase(&'a str),
    Recovery(&'a RecoveryKey),
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct Note {
    pub id: i64,
    pub title: String,
    pub body: String,
}

pub struct Notes {
    connection: Connection,
    key: Zeroizing<[u8; 32]>,
    keys_path: PathBuf,
}

impl Notes {
    pub fn create(path: &Path, passphrase: &str) -> Result<(Self, RecoveryKey), NotesError> {
        let keys_path = storage::keys_path(path)?;
        storage::require_absent(path)?;
        storage::require_absent(&keys_path)?;
        let mut key = Zeroizing::new([0; 32]);
        envelope::random(&mut key[..])?;
        let database = tempfile::Builder::new()
            .prefix(".notes-db-")
            .tempfile_in(storage::parent(path))?;
        let connection = keyed(database.path(), &key, OpenFlags::SQLITE_OPEN_READ_WRITE)?;
        configure(&connection)?;
        connection.execute_batch(&format!(
            "BEGIN IMMEDIATE; {SCHEMA}; PRAGMA user_version = 1; COMMIT;"
        ))?;
        connection
            .close()
            .map_err(|(_, error)| NotesError::from(error))?;
        database.as_file().sync_all()?;
        let keys = storage::stage(&keys_path, &envelope::wrap(passphrase, &key)?)?;
        // Check both actual staged files before publishing either destination.
        let probe = keyed(database.path(), &key, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        validate_schema(&probe)?;
        drop(probe);
        let unwrapped = envelope::unwrap(keys.path(), passphrase)?;
        if !bool::from(unwrapped.ct_eq(&key[..])) {
            return Err(NotesError::UnlockFailed);
        }
        storage::publish_new(path, &keys_path, database, keys)?;
        let notes = Self::open_key(path, Zeroizing::new(*key)).map_err(|error| {
            NotesError::IncompleteCreate {
                database: path.display().to_string(),
                keys: keys_path.display().to_string(),
                staged_database: path.display().to_string(),
                reason: error.to_string(),
            }
        })?;
        Ok((notes, RecoveryKey(key)))
    }

    pub fn open(path: &Path, unlock: Unlock<'_>) -> Result<Self, NotesError> {
        let keys_path = storage::keys_path(path)?;
        let key = match unlock {
            Unlock::Passphrase(passphrase) => envelope::unwrap(&keys_path, passphrase)?,
            Unlock::Recovery(recovery) => Zeroizing::new(*recovery.0),
        };
        Self::open_key(path, key)
    }

    fn open_key(path: &Path, key: Zeroizing<[u8; 32]>) -> Result<Self, NotesError> {
        storage::require_file(path)?;
        // A wrong recovery key must not trigger journal recovery or writable pragmas.
        let probe = key_connection(path, &key, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        match schema_count(&probe) {
            Ok(_) => validate_schema(&probe)?,
            Err(error)
                if error.sqlite_error().is_some_and(|error| {
                    error.extended_code == rusqlite::ffi::SQLITE_READONLY_ROLLBACK
                }) =>
            {
                drop(probe);
                // Validate crash recovery on encrypted copies before permitting any
                // write to the original, even when the recovery key is wrong.
                authenticate_hot_copy(path, &key)?;
                return Self::open_validated(path, key);
            }
            Err(error) => return Err(authentication_error(error)),
        }
        drop(probe);
        Self::open_validated(path, key)
    }

    fn open_validated(path: &Path, key: Zeroizing<[u8; 32]>) -> Result<Self, NotesError> {
        let connection = keyed(path, &key, OpenFlags::SQLITE_OPEN_READ_WRITE)?;
        validate_schema(&connection)?;
        configure(&connection)?;
        Ok(Self {
            connection,
            key,
            keys_path: storage::keys_path(path)?,
        })
    }

    pub fn add(&self, title: &str, body: &str) -> Result<i64, NotesError> {
        self.connection.execute(
            "INSERT INTO notes (title, body) VALUES (?1, ?2)",
            params![title, body],
        )?;
        Ok(self.connection.last_insert_rowid())
    }

    pub fn get(&self, id: i64) -> Result<Note, NotesError> {
        self.connection
            .query_row(
                "SELECT id, title, body FROM notes WHERE id = ?1",
                [id],
                read_note,
            )
            .optional()?
            .ok_or(NotesError::NotFound(id))
    }

    /// Literal, case-sensitive substring search; an empty query lists all notes.
    pub fn search(&self, query: &str) -> Result<Vec<Note>, NotesError> {
        let mut statement = self.connection.prepare("SELECT id, title, body FROM notes WHERE instr(title, ?1) > 0 OR instr(body, ?1) > 0 ORDER BY id")?;
        let rows = statement.query_map([query], read_note)?;
        rows.collect::<Result<_, _>>().map_err(Into::into)
    }

    pub fn change_passphrase(&self, old: &str, new: &str) -> Result<(), NotesError> {
        // SQLite serializes rotations across independently opened Notes handles.
        // This transaction changes no database pages; only the sidecar is replaced.
        let transaction =
            Transaction::new_unchecked(&self.connection, TransactionBehavior::Exclusive)?;
        let current = envelope::unwrap(&self.keys_path, old)?;
        if !bool::from(current.ct_eq(&self.key[..])) {
            return Err(NotesError::UnlockFailed);
        }
        self.publish_passphrase(new, transaction)
    }

    pub fn export_recovery_key(&self, passphrase: &str) -> Result<RecoveryKey, NotesError> {
        let current = envelope::unwrap(&self.keys_path, passphrase)?;
        if !bool::from(current.ct_eq(&self.key[..])) {
            return Err(NotesError::UnlockFailed);
        }
        Ok(RecoveryKey(self.key.clone()))
    }

    /// Recovery proves possession of the data key; no old passphrase or plaintext fallback.
    pub fn recover_passphrase(&self, recovery: &RecoveryKey, new: &str) -> Result<(), NotesError> {
        let transaction =
            Transaction::new_unchecked(&self.connection, TransactionBehavior::Exclusive)?;
        if !bool::from(recovery.0.ct_eq(&self.key[..])) {
            return Err(NotesError::UnlockFailed);
        }
        self.publish_passphrase(new, transaction)
    }

    fn publish_passphrase(
        &self,
        new: &str,
        transaction: Transaction<'_>,
    ) -> Result<(), NotesError> {
        let staged = storage::stage(&self.keys_path, &envelope::wrap(new, &self.key)?)?;
        staged
            .persist(&self.keys_path)
            .map_err(|error| NotesError::Io(error.error.to_string()))?;
        storage::sync_parent(&self.keys_path)
            .map_err(|error| NotesError::RotationUncertain(error.to_string()))?;
        transaction
            .commit()
            .map_err(|error| NotesError::RotationUncertain(error.to_string()))?;
        Ok(())
    }

    pub fn update(&self, id: i64, title: &str, body: &str) -> Result<(), NotesError> {
        if self.connection.execute(
            "UPDATE notes SET title=?1, body=?2 WHERE id=?3",
            params![title, body, id],
        )? == 0
        {
            return Err(NotesError::NotFound(id));
        }
        Ok(())
    }

    pub fn remove(&self, id: i64) -> Result<(), NotesError> {
        if self
            .connection
            .execute("DELETE FROM notes WHERE id=?1", [id])?
            == 0
        {
            return Err(NotesError::NotFound(id));
        }
        Ok(())
    }
}

fn read_note(row: &rusqlite::Row<'_>) -> rusqlite::Result<Note> {
    Ok(Note {
        id: row.get(0)?,
        title: row.get(1)?,
        body: row.get(2)?,
    })
}

fn keyed(path: &Path, key: &[u8; 32], flags: OpenFlags) -> Result<Connection, NotesError> {
    let connection = key_connection(path, key, flags)?;
    schema_count(&connection).map_err(authentication_error)?;
    Ok(connection)
}

fn schema_count(connection: &Connection) -> rusqlite::Result<i64> {
    connection.query_row("SELECT count(*) FROM sqlite_master", [], |row| row.get(0))
}

fn key_connection(path: &Path, key: &[u8; 32], flags: OpenFlags) -> Result<Connection, NotesError> {
    let connection = Connection::open_with_flags(path, flags | OpenFlags::SQLITE_OPEN_NO_MUTEX)?;
    let mut specification = Zeroizing::new(String::with_capacity(67));
    specification.push_str("x'");
    const HEX: &[u8; 16] = b"0123456789abcdef";
    for byte in key {
        specification.push(HEX[(byte >> 4) as usize] as char);
        specification.push(HEX[(byte & 15) as usize] as char);
    }
    specification.push('\'');
    connection.key(rusqlite::DEFAULT_NAME, specification.as_bytes(), false)?;
    connection.busy_timeout(Duration::from_secs(5))?;
    let version: String = connection.pragma_query_one(None, "cipher_version")?;
    if version.is_empty() {
        return Err(NotesError::Unsupported("SQLCipher is required".into()));
    }
    Ok(connection)
}

fn authenticate_hot_copy(path: &Path, key: &[u8; 32]) -> Result<(), NotesError> {
    let mut journal = path.as_os_str().to_owned();
    journal.push("-journal");
    let journal = PathBuf::from(journal);
    storage::require_file(&journal)?;
    let staged = tempfile::Builder::new()
        .prefix(".notes-recover-")
        .tempdir_in(storage::parent(path))?;
    let database = staged.path().join("notes.db");
    // fs::copy streams existing encrypted files; no plaintext database is created.
    std::fs::copy(path, &database)?;
    std::fs::copy(journal, staged.path().join("notes.db-journal"))?;
    let recovered = keyed(&database, key, OpenFlags::SQLITE_OPEN_READ_WRITE)?;
    validate_schema(&recovered)?;
    Ok(())
}

fn authentication_error(error: rusqlite::Error) -> NotesError {
    if error.sqlite_error_code().is_some_and(|code| {
        matches!(
            code,
            rusqlite::ErrorCode::NotADatabase | rusqlite::ErrorCode::DatabaseCorrupt
        )
    }) {
        NotesError::UnlockFailed
    } else {
        NotesError::Database(error.to_string())
    }
}

fn configure(connection: &Connection) -> Result<(), NotesError> {
    connection.pragma_update(None, "journal_mode", "DELETE")?;
    connection.pragma_update(None, "synchronous", "FULL")?;
    connection.pragma_update(None, "temp_store", "MEMORY")?;
    Ok(())
}

fn validate_schema(connection: &Connection) -> Result<(), NotesError> {
    let version: i64 = connection.pragma_query_one(None, "user_version")?;
    if version != 1 {
        return Err(NotesError::Unsupported(
            "notes schema version is not 1".into(),
        ));
    }
    let sql: Option<String> = connection
        .query_row(
            "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = 'notes'",
            [],
            |row| row.get(0),
        )
        .optional()?;
    if sql.as_deref() != Some(SCHEMA) {
        return Err(NotesError::Corrupt(
            "notes table schema does not match version 1".into(),
        ));
    }
    Ok(())
}
