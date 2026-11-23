use std::fs;
use std::str::FromStr;

use sha2::{Digest, Sha256};
use tonalli_core::notes::{Notes, NotesError, RecoveryKey, Unlock};

fn hash(path: &std::path::Path) -> Vec<u8> {
    Sha256::digest(fs::read(path).unwrap()).to_vec()
}

#[test]
fn roundtrip_after_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("notes.db");
    let (notes, _) = Notes::create(&path, "correct horse").unwrap();
    let id = notes
        .add("Preparación", "Agua potable y alimentos")
        .unwrap();
    drop(notes);
    let reopened = Notes::open(&path, Unlock::Passphrase("correct horse")).unwrap();
    let note = reopened.get(id).unwrap();
    assert_eq!(note.title, "Preparación");
    assert_eq!(note.body, "Agua potable y alimentos");
    assert_eq!(reopened.search("potable").unwrap(), vec![note]);
    assert!(reopened.search("unavailable").unwrap().is_empty());
    assert!(matches!(reopened.get(id + 1), Err(NotesError::NotFound(_))));
}

#[test]
fn wrong_passphrase_is_error_and_file_unchanged() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("notes.db");
    let (notes, _) = Notes::create(&path, "right").unwrap();
    notes.add("Private", "unchanged marker").unwrap();
    drop(notes);
    let before = (hash(&path), hash(&path.with_extension("keys")));
    assert!(matches!(
        Notes::open(&path, Unlock::Passphrase("wrong")),
        Err(NotesError::UnlockFailed)
    ));
    assert_eq!(before, (hash(&path), hash(&path.with_extension("keys"))));
}

#[test]
fn recovery_key_opens_after_passphrase_lost() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("notes.db");
    let (notes, recovery) = Notes::create(&path, "forgotten").unwrap();
    let id = notes.add("Recovery", "available offline").unwrap();
    let exported = recovery.to_string();
    assert_eq!(exported.len(), 64);
    let imported = RecoveryKey::from_str(&exported).unwrap();
    drop(notes);
    let reopened = Notes::open(&path, Unlock::Recovery(&imported)).unwrap();
    assert_eq!(reopened.get(id).unwrap().body, "available offline");
    assert!(reopened.add("New", "recovered writer").is_ok());
}

#[test]
fn file_bytes_contain_no_plaintext() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("notes.db");
    let marker = "private-note-marker-9be1fc2d";
    let (notes, recovery) = Notes::create(&path, "a secret passphrase").unwrap();
    notes.add(marker, marker).unwrap();
    // Check files while the connection is live as well as after close.
    for entry in fs::read_dir(dir.path()).unwrap() {
        let bytes = fs::read(entry.unwrap().path()).unwrap();
        assert!(!bytes
            .windows(marker.len())
            .any(|part| part == marker.as_bytes()));
        assert!(!bytes.windows(19).any(|part| part == b"a secret passphrase"));
        assert!(!bytes
            .windows(64)
            .any(|part| part == recovery.to_string().as_bytes()));
    }
    drop(notes);
    assert!(!fs::read(path).unwrap().starts_with(b"SQLite format 3\0"));
}

#[test]
fn change_passphrase_rewraps_without_rewriting_db() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("notes.db");
    let (notes, recovery) = Notes::create(&path, "old").unwrap();
    let id = notes.add("Key rotation", "same data key").unwrap();
    let before = hash(&path);
    let old_keys = fs::read(path.with_extension("keys")).unwrap();
    assert!(matches!(
        notes.change_passphrase("bad", "new"),
        Err(NotesError::UnlockFailed)
    ));
    assert_eq!(fs::read(path.with_extension("keys")).unwrap(), old_keys);
    notes.change_passphrase("old", "new").unwrap();
    assert_eq!(hash(&path), before);
    assert_ne!(fs::read(path.with_extension("keys")).unwrap(), old_keys);
    drop(notes);
    assert!(matches!(
        Notes::open(&path, Unlock::Passphrase("old")),
        Err(NotesError::UnlockFailed)
    ));
    assert_eq!(
        Notes::open(&path, Unlock::Passphrase("new"))
            .unwrap()
            .get(id)
            .unwrap()
            .body,
        "same data key"
    );
    assert!(Notes::open(&path, Unlock::Recovery(&recovery)).is_ok());
}

#[test]
fn lost_sidecar_is_recoverable_with_recovery_key() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("notes.db");
    let (notes, recovery) = Notes::create(&path, "lost").unwrap();
    let id = notes
        .add("Preserved", "data survives missing keys")
        .unwrap();
    drop(notes);
    fs::remove_file(path.with_extension("keys")).unwrap();
    assert!(matches!(
        Notes::open(&path, Unlock::Passphrase("lost")),
        Err(NotesError::MissingKeys)
    ));
    let reopened = Notes::open(&path, Unlock::Recovery(&recovery)).unwrap();
    assert_eq!(reopened.get(id).unwrap().body, "data survives missing keys");
    reopened.add("Still writable", "yes").unwrap();
    assert!(matches!(
        reopened.change_passphrase("lost", "replacement"),
        Err(NotesError::MissingKeys)
    ));
    assert!(!path.with_extension("keys").exists());
}

#[test]
fn invalid_recovery_keys_and_missing_database_are_errors_without_creation() {
    assert!(RecoveryKey::from_str("invalid").is_err());
    assert!(RecoveryKey::from_str(&"z".repeat(64)).is_err());
    assert!(RecoveryKey::from_str(&"😀".repeat(16)).is_err());
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("missing.db");
    let key = RecoveryKey::from_str(&"01".repeat(32)).unwrap();
    assert!(Notes::open(&path, Unlock::Recovery(&key)).is_err());
    assert!(!path.exists());
}

#[test]
fn corrupt_and_oversized_envelopes_are_rejected_without_writes() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("notes.db");
    let (notes, recovery) = Notes::create(&path, "right").unwrap();
    drop(notes);
    let keys = path.with_extension("keys");
    let original = fs::read(&keys).unwrap();
    let before = hash(&path);
    for bytes in [vec![], vec![0; 1_000_000], {
        let mut bytes = original.clone();
        let end = bytes.len() - 1;
        bytes[end] ^= 1;
        bytes
    }] {
        fs::write(&keys, &bytes).unwrap();
        assert!(Notes::open(&path, Unlock::Passphrase("right")).is_err());
        assert_eq!(fs::read(&keys).unwrap(), bytes);
        assert_eq!(hash(&path), before);
        assert!(Notes::open(&path, Unlock::Recovery(&recovery)).is_ok());
    }
}

#[test]
fn create_never_overwrites_existing_or_colliding_vault_files() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("notes.db");
    fs::write(&path, b"existing data").unwrap();
    assert!(Notes::create(&path, "new").is_err());
    assert_eq!(fs::read(&path).unwrap(), b"existing data");
    fs::remove_file(&path).unwrap();
    fs::write(path.with_extension("keys"), b"orphaned keys").unwrap();
    assert!(Notes::create(&path, "new").is_err());
    assert_eq!(
        fs::read(path.with_extension("keys")).unwrap(),
        b"orphaned keys"
    );
    assert!(!path.exists());
    assert!(Notes::create(&dir.path().join("direct.keys"), "new").is_err());
}

#[test]
fn stale_handle_cannot_rotate_with_a_superseded_passphrase() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("notes.db");
    let (first, _) = Notes::create(&path, "old").unwrap();
    let second = Notes::open(&path, Unlock::Passphrase("old")).unwrap();
    let before = hash(&path);
    first.change_passphrase("old", "new").unwrap();
    assert!(matches!(
        second.change_passphrase("old", "stale"),
        Err(NotesError::UnlockFailed)
    ));
    assert_eq!(hash(&path), before);
    drop((first, second));
    assert!(Notes::open(&path, Unlock::Passphrase("new")).is_ok());
}

#[test]
fn raw_data_key_opens_with_the_independent_sqlcipher_api() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("notes.db");
    let (notes, recovery) = Notes::create(&path, "wrapped passphrase").unwrap();
    notes
        .add("Interop", "raw key, not PBKDF2 of raw bytes")
        .unwrap();
    drop(notes);
    let external =
        rusqlite::Connection::open_with_flags(&path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .unwrap();
    external
        .key(rusqlite::DEFAULT_NAME, format!("x'{recovery}'"), false)
        .unwrap();
    let body: String = external
        .query_row("SELECT body FROM notes", [], |row| row.get(0))
        .unwrap();
    assert_eq!(body, "raw key, not PBKDF2 of raw bytes");
    let before = hash(&path);
    let wrong = RecoveryKey::from_str(&"01".repeat(32)).unwrap();
    assert!(matches!(
        Notes::open(&path, Unlock::Recovery(&wrong)),
        Err(NotesError::UnlockFailed)
    ));
    assert_eq!(hash(&path), before);
}

#[test]
fn unknown_profile_and_changed_authenticated_header_are_read_only_errors() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("notes.db");
    let (notes, _) = Notes::create(&path, "pass").unwrap();
    drop(notes);
    let keys = path.with_extension("keys");
    let original = fs::read(&keys).unwrap();
    let before = hash(&path);
    for field in [12..16, 16..32, 32..56] {
        let mut changed = original.clone();
        changed[field].fill(0xff);
        fs::write(&keys, &changed).unwrap();
        assert!(Notes::open(&path, Unlock::Passphrase("pass")).is_err());
        assert_eq!(fs::read(&keys).unwrap(), changed);
        assert_eq!(hash(&path), before);
    }
}

#[test]
fn simultaneous_rotations_authenticate_the_current_envelope_under_lock() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("notes.db");
    let (first, _) = Notes::create(&path, "old").unwrap();
    first.add("A", "stable database bytes").unwrap();
    let second = Notes::open(&path, Unlock::Passphrase("old")).unwrap();
    let before = hash(&path);
    let old_keys = fs::read(path.with_extension("keys")).unwrap();
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let workers: Vec<_> = [(first, "first"), (second, "second")]
        .into_iter()
        .map(|(notes, new)| {
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                (new, notes.change_passphrase("old", new))
            })
        })
        .collect();
    let results: Vec<_> = workers
        .into_iter()
        .map(|worker| worker.join().unwrap())
        .collect();
    assert_eq!(
        results.iter().filter(|(_, result)| result.is_ok()).count(),
        1
    );
    assert_eq!(
        results
            .iter()
            .filter(|(_, result)| matches!(result, Err(NotesError::UnlockFailed)))
            .count(),
        1
    );
    let winner = results.iter().find(|(_, result)| result.is_ok()).unwrap().0;
    assert!(Notes::open(&path, Unlock::Passphrase(winner)).is_ok());
    assert_eq!(hash(&path), before);
    let new_keys = fs::read(path.with_extension("keys")).unwrap();
    assert_ne!(&old_keys[16..32], &new_keys[16..32]);
    assert_ne!(&old_keys[32..56], &new_keys[32..56]);
}

#[test]
#[ignore = "private subprocess helper; the active force-stop regression invokes it"]
fn crash_writer_subprocess() {
    use std::io::Write;
    if std::env::var("TONALLI_NOTES_TEST_CRASH_WRITER").as_deref() != Ok("1") {
        return;
    }
    let Some(path) = std::env::var_os("TONALLI_NOTES_CRASH_DB") else {
        return;
    };
    let raw = std::env::var("TONALLI_NOTES_CRASH_KEY").unwrap();
    let connection =
        rusqlite::Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE)
            .unwrap();
    connection
        .key(rusqlite::DEFAULT_NAME, format!("x'{raw}'"), false)
        .unwrap();
    connection
        .execute_batch("PRAGMA cache_size = 5; BEGIN IMMEDIATE;")
        .unwrap();
    let body = "uncommitted-note-marker-52d986f1".repeat(300);
    for _ in 0..128 {
        connection
            .execute(
                "INSERT INTO notes (title, body) VALUES ('uncommitted', ?1)",
                [&body],
            )
            .unwrap();
    }
    println!("NOTES_JOURNAL_READY");
    std::io::stdout().flush().unwrap();
    loop {
        std::thread::park();
    }
}

fn kill_spilled_writer(path: &std::path::Path, recovery: &RecoveryKey) {
    use std::io::BufRead;
    use std::process::{Command, Stdio};
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "crash_writer_subprocess",
            "--ignored",
            "--nocapture",
        ])
        .env("TONALLI_NOTES_TEST_CRASH_WRITER", "1")
        .env("TONALLI_NOTES_CRASH_DB", path)
        .env("TONALLI_NOTES_CRASH_KEY", recovery.to_string())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let output = child.stdout.take().unwrap();
    let (ready, receiver) = std::sync::mpsc::channel();
    let reader = std::thread::spawn(move || {
        for line in std::io::BufReader::new(output)
            .lines()
            .map_while(Result::ok)
        {
            if line.contains("NOTES_JOURNAL_READY") {
                let _ = ready.send(());
                break;
            }
        }
    });
    let signaled = receiver
        .recv_timeout(std::time::Duration::from_secs(20))
        .is_ok();
    let _ = child.kill();
    child.wait().unwrap();
    reader.join().unwrap();
    assert!(
        signaled,
        "SQLCipher child must reach a spilled uncommitted write"
    );
}

#[test]
fn force_stop_recovers_hot_journal_without_wrong_key_mutation() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("notes.db");
    let journal = dir.path().join("notes.db-journal");
    let (notes, recovery) = Notes::create(&path, "right").unwrap();
    let id = notes.add("Committed", "survives a killed writer").unwrap();
    drop(notes);
    kill_spilled_writer(&path, &recovery);
    assert!(fs::metadata(&journal).unwrap().len() > 512);
    let marker = b"uncommitted-note-marker-52d986f1";
    assert!(!fs::read(&journal)
        .unwrap()
        .windows(marker.len())
        .any(|bytes| bytes == marker));
    let before = (
        hash(&path),
        hash(&journal),
        hash(&path.with_extension("keys")),
    );
    assert!(matches!(
        Notes::open(&path, Unlock::Passphrase("wrong")),
        Err(NotesError::UnlockFailed)
    ));
    assert_eq!(
        before,
        (
            hash(&path),
            hash(&journal),
            hash(&path.with_extension("keys"))
        )
    );
    let wrong = RecoveryKey::from_str(&"01".repeat(32)).unwrap();
    assert!(Notes::open(&path, Unlock::Recovery(&wrong)).is_err());
    assert_eq!(
        before,
        (
            hash(&path),
            hash(&journal),
            hash(&path.with_extension("keys"))
        )
    );
    let probe =
        rusqlite::Connection::open_with_flags(&path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .unwrap();
    probe
        .key(rusqlite::DEFAULT_NAME, format!("x'{recovery}'"), false)
        .unwrap();
    let error = probe
        .query_row("SELECT count(*) FROM sqlite_master", [], |row| {
            row.get::<_, i64>(0)
        })
        .unwrap_err();
    assert_eq!(
        error.sqlite_error().map(|error| error.extended_code),
        Some(rusqlite::ffi::SQLITE_READONLY_ROLLBACK)
    );
    drop(probe);
    let reopened = Notes::open(&path, Unlock::Passphrase("right")).unwrap();
    assert_eq!(reopened.get(id).unwrap().body, "survives a killed writer");
    assert!(reopened.search("uncommitted").unwrap().is_empty());
    reopened.add("After recovery", "still writable").unwrap();
}

#[test]
fn recovery_key_alone_recovers_a_separate_hot_journal() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("notes.db");
    let (notes, recovery) = Notes::create(&path, "forgotten").unwrap();
    let id = notes
        .add("Recovery", "committed before the second crash")
        .unwrap();
    drop(notes);
    kill_spilled_writer(&path, &recovery);
    fs::remove_file(path.with_extension("keys")).unwrap();
    let reopened = Notes::open(&path, Unlock::Recovery(&recovery)).unwrap();
    assert_eq!(
        reopened.get(id).unwrap().body,
        "committed before the second crash"
    );
    assert!(reopened.search("uncommitted").unwrap().is_empty());
    reopened
        .add("Recovered", "write after recovery without sidecar")
        .unwrap();
    assert!(!path.with_extension("keys").exists());
}
