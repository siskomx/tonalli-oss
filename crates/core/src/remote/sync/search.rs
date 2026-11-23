use super::*;
use crate::{
    index::{
        jobs::{IndexJobError, IndexJobs, IndexSource},
        snapshot::Manifest,
    },
    pack::PreadSource,
    server::PackId,
    zim::Zim,
};
use std::sync::Arc;

impl CopySession {
    pub async fn copy_search(
        self,
        root: PathBuf,
        action: impl Fn() -> DownloadAction + Send + 'static,
        progress: impl FnMut(DownloadProgress) -> Result<(), PackError> + Send + 'static,
    ) -> Result<CopyOutcome, CopyError> {
        self.copy_search_observed(root, action, progress, |_, _| Ok(()))
            .await
    }
    pub(crate) async fn copy_search_observed(
        self,
        root: PathBuf,
        action: impl Fn() -> DownloadAction + Send + 'static,
        progress: impl FnMut(DownloadProgress) -> Result<(), PackError> + Send + 'static,
        prepared: impl FnOnce(&crate::sync::CopyLease<'_>, &PackManifest) -> Result<(), PackError>
            + Send
            + 'static,
    ) -> Result<CopyOutcome, CopyError> {
        self.library
            .copy_search_observed(&self.id, root, action, progress, prepared)
            .await
    }
}
impl RemoteLibrary {
    async fn search_json(
        &self,
        path: &str,
        limit: usize,
    ) -> Result<serde_json::Value, RemoteError> {
        for attempt in 0..3 {
            match self.json_bounded(path, &[], limit).await {
                Err(RemoteError::Rejected(429)) if attempt < 2 => {
                    tokio::time::sleep(std::time::Duration::from_millis(100)).await
                }
                reply => return reply,
            }
        }
        Err(RemoteError::Rejected(429))
    }
    pub async fn copy_search(
        &self,
        id: &str,
        root: PathBuf,
        action: impl Fn() -> DownloadAction + Send + 'static,
        progress: impl FnMut(DownloadProgress) -> Result<(), PackError> + Send + 'static,
    ) -> Result<CopyOutcome, CopyError> {
        self.copy_search_observed(id, root, action, progress, |_, _| Ok(()))
            .await
    }
    pub(crate) async fn copy_search_observed(
        &self,
        id: &str,
        root: PathBuf,
        action: impl Fn() -> DownloadAction + Send + 'static,
        mut progress: impl FnMut(DownloadProgress) -> Result<(), PackError> + Send + 'static,
        prepared: impl FnOnce(&crate::sync::CopyLease<'_>, &PackManifest) -> Result<(), PackError>
            + Send
            + 'static,
    ) -> Result<CopyOutcome, CopyError> {
        super::super::pack_id(id)?;
        let book: PackManifest = serde_json::from_value(
            self.search_json(&format!("/api/pack-transfer/{id}"), 4 * 1024 * 1024)
                .await?,
        )
        .map_err(|_| RemoteError::InvalidResponse)?;
        book.validate()?;
        if book.id != id || book.kind != crate::catalog::CatalogKind::Zim {
            return Err(RemoteError::InvalidResponse.into());
        }
        let offer = self
            .search_json(&format!("/api/index-transfer/{id}"), 1024 * 1024)
            .await?;
        let offer = Manifest::from_slice(
            &serde_json::to_vec(&offer).map_err(|_| RemoteError::InvalidResponse)?,
        )?;
        let total = offer.validate()?;
        let origin = self.origin.clone();
        let id = id.to_owned();
        let certificate = self.certificate.clone();
        let cookie = self.content.clone();
        let source_address = self.source_address;
        let closed = self.closed.clone();
        let cancelled = tokio_util::sync::CancellationToken::new();
        let _guard = cancelled.clone().drop_guard();
        tokio::task::spawn_blocking(move || {
            let store = SyncStore::open(&root)?;
            let lease = store.copy_lease()?;
            let local = store.manifest(&book.local_id())?.ok_or_else(|| {
                PackError::NotFound("Save the book before its article search".into())
            })?;
            if local.sha256 != book.sha256 || local.bytes != book.bytes || local.kind != book.kind {
                return Err(
                    PackError::Mismatch("Saved book differs from the shared book".into()).into(),
                );
            }
            let zim = Arc::new(Zim::open(store.open_source(&local)?)?);
            offer.check_source(&zim)?;
            let native_source =
                IndexSource::registered(PackId::new(book.local_id())?, zim).map_err(job_error)?;
            prepared(&lease, &book)?;
            let control = || {
                if closed.is_cancelled() || cancelled.is_cancelled() {
                    DownloadAction::Cancel
                } else {
                    action()
                }
            };
            let stop = |command| match command {
                DownloadAction::Pause => Ok(CopyOutcome::Paused),
                _ => {
                    lease.clear_search(&book)?;
                    Ok(CopyOutcome::Cancelled)
                }
            };
            let command = control();
            if command != DownloadAction::Continue {
                return stop(command);
            }
            lease.reserve_search(&book, &offer)?;
            let client = super::blocking_client(&certificate, source_address, &cookie)?;
            let mut completed = 0;
            for file in &offer.files {
                let mut spec = store.search_spec(&book, file)?;
                let url = origin
                    .join(&format!(
                        "transfer/{id}/search/{}/{}",
                        offer.generation, file.name
                    ))
                    .map_err(|_| RemoteError::InvalidInput)?;
                if url.origin() != origin.origin() {
                    return Err(RemoteError::InvalidInput.into());
                }
                spec.url = Some(url.to_string());
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
                let outcome = loop {
                    match crate::download::fetch_pack_controlled_with_client(
                        &spec,
                        &root,
                        &client,
                        control,
                        |p| {
                            progress(DownloadProgress {
                                phase: p.phase,
                                downloaded_bytes: completed + p.downloaded_bytes,
                                total_bytes: total,
                                verified_bytes: completed + p.verified_bytes,
                            })
                        },
                    ) {
                        Err(PackError::RateLimited) if std::time::Instant::now() < deadline => {
                            for _ in 0..2 {
                                let command = control();
                                if command != DownloadAction::Continue {
                                    return stop(command);
                                }
                                std::thread::sleep(std::time::Duration::from_millis(50));
                            }
                        }
                        result => break result?,
                    }
                };
                match outcome {
                    DownloadOutcome::Paused => return Ok(CopyOutcome::Paused),
                    DownloadOutcome::Cancelled => return stop(DownloadAction::Cancel),
                    DownloadOutcome::Complete(_) => completed += file.bytes,
                }
            }
            let jobs = IndexJobs::open(&root).map_err(job_error)?;
            let stopped = std::cell::Cell::new(DownloadAction::Continue);
            let ready = jobs
                .install_snapshot(
                    &native_source,
                    &offer,
                    |file| {
                        let spec = store.search_spec(&book, file)?;
                        let path = crate::content::storage::path(store.root(), &spec.path)?;
                        Ok(Arc::new(PreadSource::open(&path)?) as Arc<dyn crate::pack::PackSource>)
                    },
                    || {
                        let command = control();
                        stopped.set(command);
                        command == DownloadAction::Continue
                    },
                )
                .map_err(job_error)?;
            match ready {
                Some(_) => {
                    lease.clear_search(&book)?;
                    Ok(CopyOutcome::Complete(local))
                }
                None => stop(stopped.get()),
            }
        })
        .await
        .map_err(|_| RemoteError::Unavailable)?
    }
}
fn job_error(error: IndexJobError) -> PackError {
    match error {
        IndexJobError::Pack(e) => e,
        _ => PackError::Mismatch("Search preparation is busy or its source changed".into()),
    }
}
