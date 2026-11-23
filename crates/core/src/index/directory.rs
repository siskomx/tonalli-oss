use std::{
    fmt, io,
    path::{Path, PathBuf},
    sync::Arc,
};

use tantivy::directory::{
    error::{DeleteError, LockError, OpenReadError, OpenWriteError},
    Directory, DirectoryLock, FileHandle, FileSlice, Lock, WatchCallback, WatchHandle, WritePtr,
};

#[derive(Clone, Debug)]
pub(super) struct DiagnosticDirectory {
    inner: Box<dyn Directory>,
}

impl DiagnosticDirectory {
    pub(super) fn new(inner: impl Directory) -> Self {
        Self {
            inner: Box::new(inner),
        }
    }
}

#[derive(Debug)]
struct DirectoryIo {
    operation: &'static str,
    path: PathBuf,
    attempts: u32,
    source: Arc<io::Error>,
}
impl fmt::Display for DirectoryIo {
    fn fmt(&self, output: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            output,
            "directory {}({:?}), attempts={}: kind={:?}, raw_os_error={:?}: {}",
            self.operation,
            self.path,
            self.attempts,
            self.source.kind(),
            self.source.raw_os_error(),
            self.source
        )
    }
}
impl std::error::Error for DirectoryIo {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(self.source.as_ref())
    }
}
fn context(
    operation: &'static str,
    path: &Path,
    source: Arc<io::Error>,
    attempts: u32,
) -> io::Error {
    io::Error::new(
        source.kind(),
        DirectoryIo {
            operation,
            path: path.to_owned(),
            attempts,
            source,
        },
    )
}

fn metadata_retry_delay(
    path: &Path,
    error: &io::Error,
    attempt: u32,
) -> Option<std::time::Duration> {
    if cfg!(windows)
        && path == Path::new(".managed.json")
        && error.raw_os_error() == Some(5)
        && (1..8).contains(&attempt)
    {
        Some(std::time::Duration::from_millis(10 << (attempt - 1)))
    } else {
        None
    }
}

impl Directory for DiagnosticDirectory {
    fn get_file_handle(&self, path: &Path) -> Result<Arc<dyn FileHandle>, OpenReadError> {
        self.inner.get_file_handle(path)
    }
    fn open_read(&self, path: &Path) -> Result<FileSlice, OpenReadError> {
        self.inner.open_read(path)
    }
    fn delete(&self, path: &Path) -> Result<(), DeleteError> {
        self.inner.delete(path)
    }
    fn exists(&self, path: &Path) -> Result<bool, OpenReadError> {
        self.inner.exists(path)
    }
    fn open_write(&self, path: &Path) -> Result<WritePtr, OpenWriteError> {
        self.inner.open_write(path).map_err(|error| match error {
            OpenWriteError::IoError { io_error, filepath } => {
                OpenWriteError::wrap_io_error(context("open_write", path, io_error, 1), filepath)
            }
            other => other,
        })
    }
    fn atomic_read(&self, path: &Path) -> Result<Vec<u8>, OpenReadError> {
        self.inner.atomic_read(path)
    }
    fn atomic_write(&self, path: &Path, data: &[u8]) -> io::Result<()> {
        let mut attempt = 1;
        loop {
            match self.inner.atomic_write(path, data) {
                Ok(()) => return Ok(()),
                Err(error) => {
                    if let Some(delay) = metadata_retry_delay(path, &error, attempt) {
                        // Retry only the complete atomic replacement with identical bytes.
                        // MmapDirectory keeps the old metadata on a failed persist.
                        std::thread::sleep(delay);
                        attempt += 1;
                    } else {
                        return Err(context("atomic_write", path, Arc::new(error), attempt));
                    }
                }
            }
        }
    }
    fn sync_directory(&self) -> io::Result<()> {
        self.inner.sync_directory()
    }
    fn acquire_lock(&self, lock: &Lock) -> Result<DirectoryLock, LockError> {
        self.inner.acquire_lock(lock)
    }
    fn watch(&self, callback: WatchCallback) -> tantivy::Result<WatchHandle> {
        self.inner.watch(callback)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        mpsc, Mutex,
    };
    use tantivy::directory::{ManagedDirectory, RamDirectory};

    #[derive(Clone, Debug)]
    struct FaultDirectory {
        inner: Box<dyn Directory>,
        atomic: bool,
        failures: Arc<AtomicUsize>,
        calls: Arc<AtomicUsize>,
        writes: Arc<Mutex<Vec<Vec<u8>>>>,
        os_code: Option<i32>,
        on_error: Option<mpsc::Sender<()>>,
    }
    impl FaultDirectory {
        fn new(atomic: bool, failures: usize, os_code: Option<i32>) -> Self {
            Self {
                inner: Box::new(RamDirectory::create()),
                atomic,
                failures: Arc::new(AtomicUsize::new(failures)),
                calls: Arc::new(AtomicUsize::new(0)),
                writes: Arc::new(Mutex::new(Vec::new())),
                os_code,
                on_error: None,
            }
        }
        fn injected_error(&self) -> io::Error {
            self.os_code
                .map(io::Error::from_raw_os_error)
                .unwrap_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::PermissionDenied,
                        "injected error without raw OS code",
                    )
                })
        }
    }
    impl Directory for FaultDirectory {
        fn get_file_handle(&self, path: &Path) -> Result<Arc<dyn FileHandle>, OpenReadError> {
            self.inner.get_file_handle(path)
        }
        fn delete(&self, path: &Path) -> Result<(), DeleteError> {
            self.inner.delete(path)
        }
        fn exists(&self, path: &Path) -> Result<bool, OpenReadError> {
            self.inner.exists(path)
        }
        fn open_write(&self, path: &Path) -> Result<WritePtr, OpenWriteError> {
            if !self.atomic {
                self.calls.fetch_add(1, Ordering::SeqCst);
                return Err(OpenWriteError::wrap_io_error(
                    self.injected_error(),
                    path.to_owned(),
                ));
            }
            self.inner.open_write(path)
        }
        fn atomic_read(&self, path: &Path) -> Result<Vec<u8>, OpenReadError> {
            self.inner.atomic_read(path)
        }
        fn atomic_write(&self, path: &Path, data: &[u8]) -> io::Result<()> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.writes.lock().unwrap().push(data.to_vec());
            let result = if self.atomic
                && self
                    .failures
                    .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |count| {
                        count.checked_sub(1)
                    })
                    .is_ok()
            {
                Err(self.injected_error())
            } else {
                self.inner.atomic_write(path, data)
            };
            if result.is_err() {
                if let Some(signal) = &self.on_error {
                    let _ = signal.send(());
                }
            }
            result
        }
        fn sync_directory(&self) -> io::Result<()> {
            self.inner.sync_directory()
        }
        fn watch(&self, callback: WatchCallback) -> tantivy::Result<WatchHandle> {
            self.inner.watch(callback)
        }
    }

    #[test]
    fn managed_registration_failure_names_atomic_write_before_outer_relabel() {
        let diagnostic = DiagnosticDirectory::new(FaultDirectory::new(true, usize::MAX, Some(5)));
        let managed = ManagedDirectory::wrap(Box::new(diagnostic)).unwrap();
        let error = managed
            .open_write(Path::new("probe.fast"))
            .err()
            .expect("injected failure");
        let text = format!("{error:?}");
        assert!(
            text.contains("atomic_write") && text.contains(".managed.json"),
            "original directory operation lost: {text}"
        );
        assert!(text.contains("probe.fast") && text.contains("code: 5"));
        if let OpenWriteError::IoError { io_error, .. } = error {
            assert_eq!(io_error.kind(), io::Error::from_raw_os_error(5).kind());
            assert_eq!(
                io_error
                    .get_ref()
                    .unwrap()
                    .downcast_ref::<DirectoryIo>()
                    .unwrap()
                    .source
                    .raw_os_error(),
                Some(5)
            );
        } else {
            panic!("injected I/O variant changed");
        }
    }

    #[test]
    fn direct_open_failure_names_open_write_instead_of_atomic_metadata() {
        let diagnostic = DiagnosticDirectory::new(FaultDirectory::new(false, usize::MAX, Some(5)));
        let managed = ManagedDirectory::wrap(Box::new(diagnostic)).unwrap();
        let error = managed
            .open_write(Path::new("probe.fast"))
            .err()
            .expect("injected failure");
        let text = format!("{error:?}");
        assert!(
            text.contains("open_write") && text.contains("probe.fast"),
            "direct directory operation lost: {text}"
        );
        assert!(!text.contains("atomic_write") && text.contains("code: 5"));
    }

    #[cfg(windows)]
    #[test]
    fn transient_managed_metadata_denial_retries_identical_bytes() {
        let fault = FaultDirectory::new(true, 2, Some(5));
        fault
            .inner
            .atomic_write(Path::new(".managed.json"), b"old metadata")
            .unwrap();
        let diagnostic = DiagnosticDirectory::new(fault.clone());
        diagnostic
            .atomic_write(Path::new(".managed.json"), b"new metadata")
            .unwrap();
        assert_eq!(fault.calls.load(Ordering::SeqCst), 3);
        assert_eq!(
            fault.inner.atomic_read(Path::new(".managed.json")).unwrap(),
            b"new metadata"
        );
        assert!(fault
            .writes
            .lock()
            .unwrap()
            .iter()
            .all(|bytes| bytes == b"new metadata"));
    }

    #[cfg(windows)]
    #[test]
    fn metadata_retry_schedule_stops_after_1270ms_and_eight_attempts() {
        let error = io::Error::from_raw_os_error(5);
        let delays: Vec<_> = (1..8)
            .map(|attempt| {
                metadata_retry_delay(Path::new(".managed.json"), &error, attempt)
                    .unwrap()
                    .as_millis()
            })
            .collect();
        assert_eq!(delays, [10, 20, 40, 80, 160, 320, 640]);
        assert_eq!(delays.iter().sum::<u128>(), 1270);
        assert!(metadata_retry_delay(Path::new(".managed.json"), &error, 8).is_none());
    }

    #[test]
    fn permanent_metadata_denial_is_bounded_and_preserves_previous_bytes() {
        let fault = FaultDirectory::new(true, usize::MAX, Some(5));
        fault
            .inner
            .atomic_write(Path::new(".managed.json"), b"old metadata")
            .unwrap();
        let diagnostic = DiagnosticDirectory::new(fault.clone());
        let error = diagnostic
            .atomic_write(Path::new(".managed.json"), b"replacement")
            .unwrap_err();
        let attempts = if cfg!(windows) { 8 } else { 1 };
        assert_eq!(fault.calls.load(Ordering::SeqCst), attempts);
        assert_eq!(
            fault.inner.atomic_read(Path::new(".managed.json")).unwrap(),
            b"old metadata"
        );
        assert_eq!(error.kind(), io::Error::from_raw_os_error(5).kind());
        let detail = error
            .get_ref()
            .unwrap()
            .downcast_ref::<DirectoryIo>()
            .unwrap();
        assert_eq!(detail.source.raw_os_error(), Some(5));
        assert!(format!("{error:?}").contains(&format!("attempts: {attempts}")));
    }

    #[test]
    fn other_paths_errors_and_direct_opens_are_not_retried() {
        for (path, code) in [
            ("meta.json", Some(5)),
            ("nested/.managed.json", Some(5)),
            (".managed.json", Some(32)),
            (".managed.json", Some(13)),
            (".managed.json", None),
        ] {
            let fault = FaultDirectory::new(true, usize::MAX, code);
            let diagnostic = DiagnosticDirectory::new(fault.clone());
            assert!(diagnostic.atomic_write(Path::new(path), b"data").is_err());
            assert_eq!(
                fault.calls.load(Ordering::SeqCst),
                1,
                "unexpected retry for {path} / {code:?}"
            );
        }
        let fault = FaultDirectory::new(false, usize::MAX, Some(5));
        let diagnostic = DiagnosticDirectory::new(fault.clone());
        assert!(diagnostic.open_write(Path::new("probe.fast")).is_err());
        assert_eq!(fault.calls.load(Ordering::SeqCst), 1);
    }

    #[cfg(not(windows))]
    #[test]
    fn non_windows_metadata_denial_is_one_attempt() {
        let fault = FaultDirectory::new(true, 2, Some(5));
        assert!(DiagnosticDirectory::new(fault.clone())
            .atomic_write(Path::new(".managed.json"), b"data")
            .is_err());
        assert_eq!(fault.calls.load(Ordering::SeqCst), 1);
    }

    #[cfg(windows)]
    #[test]
    fn released_non_delete_sharing_handle_allows_eventual_atomic_replacement() {
        use std::os::windows::fs::OpenOptionsExt;
        let root = tempfile::tempdir().unwrap();
        let metadata = root.path().join(".managed.json");
        std::fs::write(&metadata, b"old metadata").unwrap();
        let held = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(1 | 2)
            .open(&metadata)
            .unwrap();
        let (sender, receiver) = mpsc::channel();
        let mut observed = FaultDirectory::new(true, 0, Some(5));
        observed.inner = Box::new(tantivy::directory::MmapDirectory::open(root.path()).unwrap());
        observed.on_error = Some(sender);
        let release = std::thread::spawn(move || {
            receiver
                .recv_timeout(std::time::Duration::from_secs(5))
                .expect("observe real first replacement failure");
            drop(held);
        });
        let result = DiagnosticDirectory::new(observed.clone())
            .atomic_write(Path::new(".managed.json"), b"new metadata");
        release.join().unwrap();
        result.unwrap();
        assert!((2..=8).contains(&observed.calls.load(Ordering::SeqCst)));
        assert_eq!(std::fs::read(&metadata).unwrap(), b"new metadata");
        assert_eq!(
            std::fs::read_dir(root.path()).unwrap().count(),
            1,
            "failed temporary replacements were not cleaned up"
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_non_delete_sharing_handle_denies_metadata_replacement() {
        use std::os::windows::fs::OpenOptionsExt;
        let root = tempfile::tempdir().unwrap();
        let directory =
            DiagnosticDirectory::new(tantivy::directory::MmapDirectory::open(root.path()).unwrap());
        let managed = ManagedDirectory::wrap(Box::new(directory)).unwrap();
        managed.atomic_write(Path::new("seed"), b"seed").unwrap();
        let metadata = root.path().join(".managed.json");
        let original = std::fs::read(&metadata).unwrap();
        let held = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(1 | 2)
            .open(&metadata)
            .unwrap();
        let error = managed
            .open_write(Path::new("probe.fast"))
            .err()
            .expect("delete sharing was intentionally withheld");
        let text = format!("{error:?}");
        assert!(
            text.contains("atomic_write") && text.contains(".managed.json"),
            "{text}"
        );
        if let OpenWriteError::IoError { io_error, .. } = &error {
            let detail = io_error
                .get_ref()
                .unwrap()
                .downcast_ref::<DirectoryIo>()
                .unwrap();
            assert!(
                matches!(detail.source.raw_os_error(), Some(5 | 32)),
                "{text}"
            );
        } else {
            panic!("unexpected error: {text}");
        }
        assert_eq!(std::fs::read(&metadata).unwrap(), original);
        assert!(!root.path().join("probe.fast").exists());
        assert_eq!(
            std::fs::read_dir(root.path()).unwrap().count(),
            2,
            "failed temporary replacements must leave only original metadata and seed"
        );
        eprintln!("synthetic non-delete-sharing denial (not attribution of R2): {text}");
        drop(held);
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "explicit bounded synthetic LocalAppData churn; retains all output and logs"]
    fn localappdata_synthetic_directory_churn() {
        use std::io::Write;
        use tantivy::{doc, Index};
        let label = std::env::var("TONALLI_INDEX_DIAGNOSTIC_LABEL")
            .expect("set a new owned diagnostic label");
        assert!(
            !label.is_empty()
                && label.len() <= 49
                && label
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
        );
        let appdata = PathBuf::from(std::env::var_os("LOCALAPPDATA").expect("LocalAppData"));
        let parent = appdata.join("Tonalli/index-diagnostics");
        std::fs::create_dir_all(&parent).unwrap();
        assert!(parent
            .canonicalize()
            .unwrap()
            .starts_with(appdata.canonicalize().unwrap()));
        let root = parent.join(&label);
        std::fs::create_dir(&root).expect("diagnostic output must be new; never overwrite");
        let mut log = std::fs::File::create_new(root.join("probe.jsonl")).unwrap();
        let started = std::time::Instant::now();
        let result = (|| -> Result<(u64, usize, usize), crate::pack::PackError> {
            let index_path = root.join("index");
            std::fs::create_dir(&index_path)?;
            let (schema, title, body, url) = super::super::schema();
            let directory = tantivy::directory::MmapDirectory::open(&index_path)
                .map_err(super::super::index_error)?;
            let index = Index::create(
                DiagnosticDirectory::new(directory),
                schema,
                tantivy::IndexSettings::default(),
            )
            .map_err(super::super::index_error)?;
            let raw_writer = index
                .writer_with_num_threads(1, super::super::WRITER_BUDGET)
                .map_err(super::super::index_error)?;
            // Synthetic probe only: 2 MiB drains exercise metadata churn in <=512 MiB;
            // production indexing keeps its unchanged 64 MiB queue budget.
            let mut writer = super::super::writer::BuildWriter::new(raw_writer, 2 * 1024 * 1024);
            let text = "synthetic offline knowledge alpha beta gamma delta ".repeat(160);
            let mut generated = 0usize;
            let mut documents = 0u64;
            for batch in 0..128 {
                for row in 0..256 {
                    let name = format!("Synthetic {batch}-{row}");
                    let path = format!("C/Synthetic_{batch}_{row}");
                    let bytes = text.len() + name.len() + path.len();
                    generated += bytes;
                    if generated > 512 * 1024 * 1024 {
                        return Err(crate::pack::PackError::Mismatch(
                            "synthetic text cap exceeded".into(),
                        ));
                    }
                    writer.submit(
                        doc!(title => name, body => text.clone(), url => path),
                        bytes,
                    )?;
                    documents += 1;
                }
                writeln!(
                    log,
                    "{}",
                    serde_json::json!({"batch":batch+1,"documents":documents,"generated_bytes":generated,"elapsed_ms":started.elapsed().as_millis()})
                )?;
                log.flush()?;
            }
            let drains = writer.finish()?;
            if !(120..=136).contains(&drains) {
                return Err(crate::pack::PackError::Corrupt(format!(
                    "unexpected synthetic drain count {drains}"
                )));
            }
            if index
                .reader()
                .map_err(super::super::index_error)?
                .searcher()
                .num_docs()
                != documents
            {
                return Err(crate::pack::PackError::Corrupt(
                    "synthetic committed document count differs".into(),
                ));
            }
            Ok((documents, generated, drains))
        })();
        let record = match &result {
            Ok((documents, generated, drains)) => {
                serde_json::json!({"status":"complete","synthetic":true,"documents":documents,"generated_bytes":generated,"drains":drains,"elapsed_ms":started.elapsed().as_millis()})
            }
            Err(error) => {
                serde_json::json!({"status":"failed","synthetic":true,"error":error.to_string(),"elapsed_ms":started.elapsed().as_millis()})
            }
        };
        writeln!(log, "{record}").unwrap();
        log.sync_all().unwrap();
        eprintln!(
            "synthetic diagnostic retained at {}: {record}",
            root.display()
        );
        result.unwrap();
    }
}
