use super::ranges::{self, Snapshot};
use crate::pack::PackError;
use std::{
    ffi::{c_char, c_void, CString},
    fs::File,
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
};

type Extract = unsafe extern "C" fn(
    *mut c_void,
    usize,
    usize,
    *const c_char,
    *const c_char,
    *const c_char,
    i32,
    i32,
    *mut u64,
    *mut u64,
) -> i32;
// Go owns runtime threads for the process lifetime; never unload this library.
#[cfg(not(target_os = "ios"))]
static LIBRARY: OnceLock<Result<libloading::Library, String>> = OnceLock::new();

// iPhone apps cannot load a shared library they ship, so the Go extractor is linked in
// as a static archive (see ios/build.rs) and called directly.
#[cfg(target_os = "ios")]
unsafe extern "C" {
    fn TonalliExtractMap(
        owner: *mut c_void,
        open: usize,
        plan: usize,
        region: *const c_char,
        bbox: *const c_char,
        output: *const c_char,
        zoom: i32,
        dry: i32,
        output_bytes: *mut u64,
        transfer_bytes: *mut u64,
    ) -> i32;
}

#[cfg(target_os = "ios")]
fn extractor() -> Result<Extract, PackError> {
    Ok(TonalliExtractMap)
}

#[cfg(not(target_os = "ios"))]
fn extractor() -> Result<Extract, PackError> {
    let path = library_path()?;
    let library = LIBRARY
        .get_or_init(|| {
            unsafe { libloading::Library::new(path) }
                .map_err(|_| "Map extractor unavailable".into())
        })
        .as_ref()
        .map_err(|error| PackError::NotFound(error.clone()))?;
    let function = unsafe { library.get::<Extract>(b"TonalliExtractMap\0") }
        .map_err(|_| PackError::Corrupt("Map extractor interface".into()))?;
    // The library is never unloaded, so the address stays valid for the process lifetime.
    Ok(*function)
}

struct Owner<'a> {
    root: &'a Path,
    source: &'a Snapshot,
    client: &'a reqwest::blocking::Client,
    state: Mutex<Callbacks<'a>>,
}
struct Callbacks<'a> {
    progress: &'a mut (dyn FnMut(u64) -> Result<(), PackError> + Send),
    plan: &'a mut (dyn FnMut(u64, u64) -> Result<(), PackError> + Send),
    failure: Option<PackError>,
}

unsafe extern "C" fn open(owner: *mut c_void, offset: i64, length: i64) -> u64 {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        // The synchronous extractor joins its workers before returning; Owner outlives callbacks.
        let owner = unsafe { &*(owner as *const Owner<'_>) };
        let Ok(mut state) = owner.state.lock() else {
            return u64::MAX;
        };
        if state.failure.is_some() {
            return u64::MAX;
        }
        let result = if offset < 0 || length <= 0 {
            Err(PackError::Truncated)
        } else {
            ranges::open_range(
                owner.root,
                owner.source,
                owner.client,
                offset as u64,
                length as u64,
                &mut state.progress,
            )
        };
        match result {
            Ok(file) => raw_handle(file),
            Err(error) => {
                state.failure = Some(error);
                u64::MAX
            }
        }
    }))
    .unwrap_or(u64::MAX)
}

unsafe extern "C" fn plan(owner: *mut c_void, output: u64, transfer: u64) -> i32 {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let owner = unsafe { &*(owner as *const Owner<'_>) };
        let Ok(mut state) = owner.state.lock() else {
            return 1;
        };
        if state.failure.is_some() {
            return 1;
        }
        match (state.plan)(output, transfer) {
            Ok(()) => 0,
            Err(error) => {
                state.failure = Some(error);
                1
            }
        }
    }))
    .unwrap_or(1)
}

#[cfg(windows)]
fn raw_handle(file: File) -> u64 {
    use std::os::windows::io::IntoRawHandle;
    file.into_raw_handle() as usize as u64
}
#[cfg(unix)]
fn raw_handle(file: File) -> u64 {
    use std::os::fd::IntoRawFd;
    file.into_raw_fd() as u64
}

#[cfg(not(target_os = "ios"))]
fn library_path() -> Result<PathBuf, PackError> {
    #[cfg(target_os = "android")]
    {
        Ok(PathBuf::from("libtonalli_maps.so"))
    }
    #[cfg(not(target_os = "android"))]
    {
        Ok(std::env::current_exe()?
            .parent()
            .ok_or_else(|| PackError::NotFound("Map extractor".into()))?
            .join("tonalli_maps.dll"))
    }
}

pub(super) fn run(
    root: &Path,
    source: &Snapshot,
    client: &reqwest::blocking::Client,
    region: &Path,
    output: &Path,
    zoom: u8,
    dry: bool,
    progress: &mut (dyn FnMut(u64) -> Result<(), PackError> + Send),
    preflight: &mut (dyn FnMut(u64, u64) -> Result<(), PackError> + Send),
) -> Result<(u64, u64), PackError> {
    let function = extractor()?;
    let text = |path: &Path| {
        CString::new(
            path.to_str()
                .ok_or_else(|| PackError::Mismatch("Map path encoding".into()))?,
        )
        .map_err(|_| PackError::Mismatch("Map path encoding".into()))
    };
    let region = text(region)?;
    let output = text(output)?;
    let bbox = CString::default();
    let owner = Owner {
        root,
        source,
        client,
        state: Mutex::new(Callbacks {
            progress,
            plan: preflight,
            failure: None,
        }),
    };
    let mut size = 0;
    let mut transfer = 0;
    let status = unsafe {
        function(
            &owner as *const _ as *mut c_void,
            open as *const () as usize,
            plan as *const () as usize,
            region.as_ptr(),
            bbox.as_ptr(),
            output.as_ptr(),
            i32::from(zoom),
            i32::from(dry),
            &mut size,
            &mut transfer,
        )
    };
    let state = owner
        .state
        .into_inner()
        .map_err(|_| PackError::Corrupt("Map worker callback".into()))?;
    if let Some(error) = state.failure {
        return Err(error);
    }
    if status != 0 {
        return Err(PackError::Corrupt("Map extraction could not finish".into()));
    }
    Ok((size, transfer))
}
