use std::fs::OpenOptions;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock, Weak};

use llama_cpp_2::llama_backend::LlamaBackend;
use llama_cpp_2::model::params::LlamaModelParams;
use llama_cpp_2::model::LlamaModel;

use super::gguf::preflight;
use super::memory::{self, Allocation, Budget};
use crate::pack::{PackError, PackSource, PreadSource};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModelKind {
    Generation,
    Embedding,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelInfo {
    pub kind: ModelKind,
    pub file_bytes: u64,
    pub training_context_tokens: u32,
    pub embedding_dimensions: u32,
    /// Provisional admission estimate, not measured resident memory.
    pub estimated_reserved_bytes: u64,
}

pub(super) struct Resources {
    // Native memory is freed before the file lease and budget reservation.
    pub native: LlamaModel,
    _input: PreadSource,
    _installed_source: Option<Arc<dyn PackSource>>,
    reservation: Mutex<Reservation>,
    pub info: ModelInfo,
}

pub(super) struct OpenModel {
    pub resources: Arc<Resources>,
    pub cancel: Option<Weak<AtomicBool>>,
}

impl Drop for OpenModel {
    fn drop(&mut self) {
        if let Some(flag) = self.cancel.as_ref().and_then(Weak::upgrade) {
            flag.store(true, Ordering::Release);
        }
    }
}

static BACKEND: OnceLock<Result<LlamaBackend, PackError>> = OnceLock::new();
static BUDGET: Mutex<Budget> = Mutex::new(Budget {
    total: 0,
    uncommitted: 0,
    policy: memory::Policy::PLATFORM,
});

struct Reservation {
    allocation: Allocation,
    resident: bool,
    context_claimed: bool,
}

fn poisoned() -> PackError {
    PackError::Corrupt("AI model or memory lock poisoned".into())
}

impl Reservation {
    fn acquire(file_bytes: u64, kind: ModelKind) -> Result<Self, PackError> {
        let mut budget = BUDGET.lock().map_err(|_| poisoned())?;
        // Read actual availability while accounting transitions are serialized.
        let allocation = budget.reserve(memory::snapshot()?, file_bytes, kind)?;
        Ok(Self {
            allocation,
            resident: false,
            context_claimed: false,
        })
    }

    fn loaded(&mut self) -> Result<(), PackError> {
        if !self.resident {
            BUDGET
                .lock()
                .map_err(|_| poisoned())?
                .resident(self.allocation)?;
            self.resident = true;
        }
        Ok(())
    }
}

impl Drop for Reservation {
    fn drop(&mut self) {
        if let Ok(mut budget) = BUDGET.lock() {
            let _ = budget.release(self.allocation, self.resident);
        }
    }
}

pub struct AiHandle {
    pub(super) inner: Mutex<Option<OpenModel>>,
}

impl AiHandle {
    #[cfg(test)]
    pub(crate) fn closed_for_test() -> Arc<Self> {
        Arc::new(Self {
            inner: Mutex::new(None),
        })
    }

    pub fn info(&self) -> Result<ModelInfo, PackError> {
        self.inner
            .lock()
            .map_err(|_| poisoned())?
            .as_ref()
            .map(|model| model.resources.info.clone())
            .ok_or_else(|| PackError::NotFound("closed AI model".into()))
    }

    pub fn close(&self) -> Result<(), PackError> {
        let model = self.inner.lock().map_err(|_| poisoned())?.take();
        drop(model);
        Ok(())
    }
}

pub(super) struct Workspace<'a> {
    resources: &'a Resources,
    resident: bool,
}

impl Resources {
    pub fn workspace(&self) -> Result<Workspace<'_>, PackError> {
        let mut reservation = self.reservation.lock().map_err(|_| poisoned())?;
        if reservation.context_claimed {
            return Err(PackError::Mismatch("AI context is busy".into()));
        }
        BUDGET
            .lock()
            .map_err(|_| poisoned())?
            .context_admission(memory::snapshot()?)?;
        reservation.context_claimed = true;
        Ok(Workspace {
            resources: self,
            resident: false,
        })
    }
}

impl Workspace<'_> {
    pub fn loaded(&mut self) -> Result<(), PackError> {
        let reservation = self.resources.reservation.lock().map_err(|_| poisoned())?;
        BUDGET
            .lock()
            .map_err(|_| poisoned())?
            .context_resident(reservation.allocation)?;
        self.resident = true;
        Ok(())
    }
}

impl Drop for Workspace<'_> {
    fn drop(&mut self) {
        if let Ok(mut reservation) = self.resources.reservation.lock() {
            if self.resident {
                if let Ok(mut budget) = BUDGET.lock() {
                    let _ = budget.context_released(reservation.allocation);
                }
            }
            reservation.context_claimed = false;
        }
    }
}

pub(super) fn backend() -> Result<&'static LlamaBackend, PackError> {
    BACKEND
        .get()
        .ok_or_else(|| PackError::Corrupt("AI backend unavailable".into()))?
        .as_ref()
        .map_err(Clone::clone)
}

/// Caller guarantees an already checksum-verified, immutable app-owned internal
/// installation, as for `pack_open_internal`. Removable paths are not inputs.
/// M1 admits single-file GGUF only; split models need whole-pack verification.
pub fn load_internal(path: &Path, kind: ModelKind) -> Result<Arc<AiHandle>, PackError> {
    load(path, kind, None)
}

/// Resolve signed app-owned content and retain its removal lease through the last
/// native borrower, including a generation finishing after handle close.
pub fn load_managed(
    store: &crate::content::ManagedStore,
    id: &str,
    kind: ModelKind,
) -> Result<Arc<AiHandle>, PackError> {
    let file = store.open_installed_file(id)?;
    if file.metadata.kind != crate::catalog::CatalogKind::Model {
        return Err(PackError::Mismatch(
            "Selected content is not a model".into(),
        ));
    }
    require_internal_storage(&file.path)?;
    crate::pack::verify_sha256(&file.path, &file.metadata.sha256)?;
    load(&file.path, kind, Some(file.source))
}

fn load(
    path: &Path,
    kind: ModelKind,
    installed_source: Option<Arc<dyn PackSource>>,
) -> Result<Arc<AiHandle>, PackError> {
    let io = |error: std::io::Error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            PackError::NotFound("AI model".into())
        } else {
            PackError::Io(error.to_string())
        }
    };
    if !std::fs::metadata(path).map_err(io)?.is_file() {
        return Err(PackError::Mismatch(
            "AI model must be an ordinary file".into(),
        ));
    }
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(1); // FILE_SHARE_READ: keep writers/deleters out.
    }
    let input = PreadSource::from_file(options.open(path).map_err(io)?)?;
    preflight(&input)?;
    let mut reservation = Reservation::acquire(input.len(), kind)?;
    let backend = BACKEND
        .get_or_init(|| {
            LlamaBackend::init()
                .map_err(|error| PackError::Corrupt(format!("AI backend initialization: {error}")))
        })
        .as_ref()
        .map_err(Clone::clone)?;
    let native = LlamaModel::load_from_file(backend, path, &cpu_params())
        .map_err(|error| PackError::Corrupt(format!("AI model load: {error}")))?;
    let training_context_tokens = native.n_ctx_train();
    let embedding_dimensions = u32::try_from(native.n_embd_out())
        .map_err(|_| PackError::Corrupt("invalid AI model dimensions".into()))?;
    if training_context_tokens == 0 || embedding_dimensions == 0 {
        return Err(PackError::Corrupt("invalid AI model dimensions".into()));
    }
    let info = ModelInfo {
        kind,
        file_bytes: input.len(),
        training_context_tokens,
        embedding_dimensions,
        estimated_reserved_bytes: reservation.allocation.total,
    };
    reservation.loaded()?;
    Ok(Arc::new(AiHandle {
        inner: Mutex::new(Some(OpenModel {
            resources: Arc::new(Resources {
                native,
                _input: input,
                _installed_source: installed_source,
                reservation: Mutex::new(reservation),
                info,
            }),
            cancel: None,
        })),
    }))
}

/// Native callers supply a canonical app-owned path; Windows models require a fixed local volume.
#[cfg(windows)]
pub fn require_internal_storage(path: &Path) -> Result<(), PackError> {
    use std::os::windows::ffi::OsStrExt;
    use std::path::{Component, Prefix};
    let drive = match path.components().next() {
        Some(Component::Prefix(prefix)) => match prefix.kind() {
            Prefix::Disk(letter) | Prefix::VerbatimDisk(letter) => letter,
            _ => {
                return Err(PackError::Mismatch(
                    "AI requires fixed local internal storage".into(),
                ))
            }
        },
        _ => {
            return Err(PackError::Mismatch(
                "AI requires fixed local internal storage".into(),
            ))
        }
    };
    let root = std::ffi::OsString::from(format!("{}:\\", char::from(drive)));
    let wide: Vec<u16> = root.encode_wide().chain(Some(0)).collect();
    #[link(name = "kernel32")]
    extern "system" {
        fn GetDriveTypeW(root: *const u16) -> u32;
    }
    // SAFETY: NUL-terminated drive root lives for the read-only Win32 call.
    if unsafe { GetDriveTypeW(wide.as_ptr()) } != 3 {
        return Err(PackError::Mismatch(
            "AI requires fixed local internal storage".into(),
        ));
    }
    Ok(())
}

#[cfg(not(windows))]
pub fn require_internal_storage(_path: &Path) -> Result<(), PackError> {
    Ok(())
}

fn cpu_params() -> LlamaModelParams {
    LlamaModelParams::default()
        .with_n_gpu_layers(0)
        .with_use_mmap(false)
        .with_use_mlock(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pack::{PackSource, PreadSource};

    #[test]
    #[ignore = "requires the already verified optional BGE model; no network fallback"]
    fn real_managed_model_lease_spans_native_borrow_after_handle_close() {
        use crate::{
            catalog::Catalog,
            content::{ManagedStore, RemoveOutcome},
        };
        use ring::signature::{Ed25519KeyPair, KeyPair};
        let source = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../docs/spike/packs/ai/bge-small-en-v1.5-q8_0.gguf");
        let sha = "f046db1dc724cf4f6f0a0c5917e922823b73eb1d27b8f9a9c2797f7866974804";
        crate::pack::verify_sha256(&source, sha).unwrap();
        let bytes = std::fs::read(&source).unwrap();
        let signer = Ed25519KeyPair::from_seed_unchecked(&[23; 32]).unwrap();
        let payload = serde_json::to_vec(&serde_json::json!({"schema_version":1,"revision":1,"entries":[{
            "id":"model","title":"Checked embedding model","kind":"model","language":"en","version":"1",
            "bytes":bytes.len(),"sha256":sha,"url":"https://example.invalid/model.gguf"
        }]})).unwrap();
        let catalog = Catalog::verify(
            &payload,
            signer.sign(&payload).as_ref(),
            signer.public_key().as_ref(),
            0,
        )
        .unwrap();
        let root = tempfile::tempdir().unwrap();
        let store = ManagedStore::open(root.path(), signer.public_key().as_ref()).unwrap();
        let spec = store.cache_spec(&catalog, "model").unwrap();
        let cache = crate::download::pack_path(root.path(), &spec.path).unwrap();
        std::fs::create_dir_all(cache.parent().unwrap()).unwrap();
        std::fs::write(cache, bytes).unwrap();
        store.install_cached(&catalog, "model").unwrap();
        let model = load_managed(&store, "model", ModelKind::Embedding).unwrap();
        let borrower = model
            .inner
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .resources
            .clone();
        model.close().unwrap();
        assert_eq!(store.remove("model").unwrap(), RemoveOutcome::InUse);
        drop(borrower);
        assert_eq!(store.remove("model").unwrap(), RemoveOutcome::Removed);
        crate::pack::verify_sha256(&source, sha).unwrap();
    }

    fn header(version: u32, tensors: u64, metadata: u64, length: usize) -> Vec<u8> {
        let mut bytes = b"GGUF".to_vec();
        bytes.extend(version.to_le_bytes());
        bytes.extend(tensors.to_le_bytes());
        bytes.extend(metadata.to_le_bytes());
        bytes.resize(length, 0);
        bytes
    }

    fn check(bytes: &[u8]) -> Result<(), PackError> {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("internal-verified-fixture.gguf");
        std::fs::write(&path, bytes).unwrap();
        preflight(&PreadSource::open(&path).unwrap())
    }

    fn string(bytes: &[u8]) -> Vec<u8> {
        let mut value = (bytes.len() as u64).to_le_bytes().to_vec();
        value.extend(bytes);
        value
    }

    fn metadata(entries: &[(&[u8], u32, Vec<u8>)]) -> Vec<u8> {
        let mut bytes = header(3, 1, entries.len() as u64, 24);
        for (key, kind, value) in entries {
            bytes.extend(string(key));
            bytes.extend(kind.to_le_bytes());
            bytes.extend(value);
        }
        bytes.extend([0; 32]); // Native validates tensor semantics after preflight.
        bytes
    }

    #[test]
    fn split_metadata_is_refused_before_backend_even_with_an_ordinary_filename() {
        let bytes = metadata(&[(b"split.count", 2, 2_u16.to_le_bytes().to_vec())]);
        // The first assertion keeps the RED run out of native code on unsafe metadata.
        assert!(matches!(check(&bytes), Err(PackError::Mismatch(_))));
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("ordinary-model.gguf");
        std::fs::write(&path, bytes).unwrap();
        assert!(BACKEND.get().is_none());
        assert!(matches!(
            load_internal(&path, ModelKind::Generation),
            Err(PackError::Mismatch(_))
        ));
        assert!(BACKEND.get().is_none());
    }

    #[test]
    fn duplicate_and_nul_metadata_keys_cannot_hide_split_count() {
        for bytes in [
            metadata(&[
                (b"split.count", 2, 1_u16.to_le_bytes().to_vec()),
                (b"split.count", 2, 2_u16.to_le_bytes().to_vec()),
            ]),
            metadata(&[(b"split.count\0suffix", 2, 2_u16.to_le_bytes().to_vec())]),
            metadata(&[(b"other\0key", 0, vec![1])]),
        ] {
            assert!(matches!(check(&bytes), Err(PackError::Corrupt(_))));
        }
    }

    #[test]
    fn split_count_requires_the_native_uint16_type() {
        for (kind, value) in [
            (0, vec![1]),
            (4, 1_u32.to_le_bytes().to_vec()),
            (8, string(b"1")),
        ] {
            assert!(matches!(
                check(&metadata(&[(b"split.count", kind, value)])),
                Err(PackError::Corrupt(_))
            ));
        }
        check(&metadata(&[(
            b"split.count",
            2,
            1_u16.to_le_bytes().to_vec(),
        )]))
        .unwrap();
    }

    #[test]
    fn unknown_nested_and_oversized_metadata_are_typed_errors() {
        let mut nested = 9_u32.to_le_bytes().to_vec();
        nested.extend(1_u64.to_le_bytes());
        let mut huge_array = 8_u32.to_le_bytes().to_vec();
        huge_array.extend(u64::MAX.to_le_bytes());
        for bytes in [
            metadata(&[(b"other", 99, vec![0])]),
            metadata(&[(b"other", 9, nested)]),
            metadata(&[(b"other", 9, huge_array)]),
            metadata(&[(b"other", 8, u64::MAX.to_le_bytes().to_vec())]),
            metadata(&[(&vec![b'a'; 1025], 0, vec![1])]),
        ] {
            assert!(check(&bytes).is_err());
        }
    }

    #[test]
    fn truncated_metadata_and_remaining_tensor_table_are_refused() {
        let mut truncated_value =
            metadata(&[(b"general.name", 8, 1000_u64.to_le_bytes().to_vec())]);
        assert!(matches!(check(&truncated_value), Err(PackError::Truncated)));
        truncated_value = metadata(&[(b"general.architecture", 8, string(b"qwen3"))]);
        truncated_value.truncate(truncated_value.len() - 32);
        assert!(matches!(check(&truncated_value), Err(PackError::Truncated)));
    }

    #[test]
    fn large_tokenizer_string_arrays_remain_accepted() {
        let mut tokens = 8_u32.to_le_bytes().to_vec();
        tokens.extend(151_936_u64.to_le_bytes());
        for _ in 0..151_936 {
            tokens.extend(string(b"x"));
        }
        check(&metadata(&[(b"tokenizer.ggml.tokens", 9, tokens)])).unwrap();
    }

    #[test]
    fn metadata_scan_span_and_positioned_read_buffers_are_bounded() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        struct VirtualSource {
            prefix: Vec<u8>,
            largest: AtomicUsize,
        }
        impl PackSource for VirtualSource {
            fn len(&self) -> u64 {
                128 << 20
            }
            fn read_at(&self, offset: u64, buffer: &mut [u8]) -> Result<usize, PackError> {
                self.largest.fetch_max(buffer.len(), Ordering::Relaxed);
                if offset
                    .checked_add(buffer.len() as u64)
                    .is_none_or(|end| end > self.len())
                {
                    return Err(PackError::Truncated);
                }
                buffer.fill(0);
                let start = usize::try_from(offset).map_err(|_| PackError::Truncated)?;
                if let Some(tail) = self.prefix.get(start..) {
                    let count = tail.len().min(buffer.len());
                    buffer[..count].copy_from_slice(&tail[..count]);
                }
                Ok(buffer.len())
            }
        }
        let source = VirtualSource {
            prefix: metadata(&[(b"large.value", 8, (65_u64 << 20).to_le_bytes().to_vec())]),
            largest: AtomicUsize::new(0),
        };
        assert!(preflight(&source).is_err());
        assert!(source.largest.load(Ordering::Relaxed) <= 64 * 1024);
    }

    #[test]
    fn truncated_header_is_a_typed_error() {
        for bytes in [vec![], b"GGUF".to_vec(), header(3, 1, 1, 23)] {
            assert!(matches!(check(&bytes), Err(PackError::Truncated)));
        }
    }

    #[test]
    fn bad_magic_and_empty_model_are_typed_corruption() {
        assert!(matches!(check(&[0; 24]), Err(PackError::Corrupt(_))));
        assert!(matches!(
            check(&header(3, 0, 0, 24)),
            Err(PackError::Corrupt(_))
        ));
    }

    #[test]
    fn unsupported_version_and_unbounded_counts_are_refused() {
        assert!(matches!(
            check(&header(99, 1, 1, 64)),
            Err(PackError::Mismatch(_))
        ));
        assert!(check(&header(3, u64::MAX, 1, 64)).is_err());
        assert!(check(&header(3, 1, u64::MAX, 64)).is_err());
        assert!(check(&header(3, 1, 1, 24)).is_err());
    }

    #[test]
    fn bounded_header_can_proceed_to_native_validation() {
        check(&metadata(&[(b"general.architecture", 8, string(b"qwen3"))])).unwrap();
    }

    #[test]
    fn invalid_input_does_not_initialize_the_backend() {
        let temp = tempfile::tempdir().unwrap();
        assert!(BACKEND.get().is_none());
        assert!(matches!(
            load_internal(&temp.path().join("missing.gguf"), ModelKind::Generation),
            Err(PackError::NotFound(_))
        ));
        assert!(load_internal(temp.path(), ModelKind::Generation).is_err());
        let corrupt = temp.path().join("corrupt.gguf");
        std::fs::write(&corrupt, [0; 24]).unwrap();
        assert!(load_internal(&corrupt, ModelKind::Generation).is_err());
        assert!(BACKEND.get().is_none());
    }

    #[test]
    fn native_load_parameters_are_cpu_only_and_never_map_input() {
        let params = cpu_params();
        assert_eq!(params.n_gpu_layers(), 0);
        assert!(!params.use_mmap());
        assert!(!params.use_mlock());
        assert!(BACKEND.get().is_none());
    }

    #[test]
    fn close_is_idempotent_and_invalidates_aliases() {
        let handle = Arc::new(AiHandle {
            inner: Mutex::new(None),
        });
        let alias = Arc::clone(&handle);
        handle.close().unwrap();
        handle.close().unwrap();
        assert!(matches!(alias.info(), Err(PackError::NotFound(_))));
    }

    #[test]
    #[ignore = "requires verified immutable internal Qwen model and actual runtime RAM admission"]
    fn real_qwen_load_info_and_close_aliases() {
        let path =
            std::env::var("TONALLI_TEST_QWEN_INTERNAL").expect("set verified internal Qwen path");
        let handle = load_internal(Path::new(&path), ModelKind::Generation).unwrap();
        let alias = Arc::clone(&handle);
        let info = handle.info().unwrap();
        assert!(
            info.file_bytes > 0
                && info.training_context_tokens > 0
                && info.embedding_dimensions > 0
        );
        handle.close().unwrap();
        assert!(matches!(alias.info(), Err(PackError::NotFound(_))));
        alias.close().unwrap();
    }
}
