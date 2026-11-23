use super::{RemoteError, RemoteLibrary};
use crate::{
    download::{DownloadAction, DownloadOutcome, DownloadProgress},
    pack::PackError,
    sync::{PackManifest, SyncStore},
};
use std::path::PathBuf;
mod search;

/// In-memory authority for one selected pack, independent of the reader view.
pub struct CopySession {
    library: RemoteLibrary,
    id: String,
}
impl CopySession {
    pub async fn copy_pack(
        self,
        root: PathBuf,
        action: impl Fn() -> DownloadAction + Send + 'static,
        progress: impl FnMut(DownloadProgress) -> Result<(), PackError> + Send + 'static,
    ) -> Result<CopyOutcome, CopyError> {
        self.copy_pack_observed(root, action, progress, |_, _| Ok(()))
            .await
    }
    pub(crate) async fn copy_pack_observed(
        self,
        root: PathBuf,
        action: impl Fn() -> DownloadAction + Send + 'static,
        progress: impl FnMut(DownloadProgress) -> Result<(), PackError> + Send + 'static,
        prepared: impl FnOnce(&crate::sync::CopyLease<'_>, &PackManifest) -> Result<(), PackError>
            + Send
            + 'static,
    ) -> Result<CopyOutcome, CopyError> {
        self.library
            .copy_pack_observed(&self.id, root, action, progress, prepared)
            .await
    }
}

#[derive(Debug, thiserror::Error)]
pub enum CopyError {
    #[error("The nearby library is unavailable")]
    Remote(#[from] RemoteError),
    #[error("The library copy could not be installed")]
    Pack(#[from] PackError),
}
#[derive(Debug)]
pub enum CopyOutcome {
    Complete(PackManifest),
    Paused,
    Cancelled,
}
impl RemoteLibrary {
    pub fn copy_session(&self, id: &str) -> Result<CopySession, RemoteError> {
        self.copy_session_on(id, None)
    }
    pub(crate) fn copy_session_on(
        &self,
        id: &str,
        source: Option<std::net::IpAddr>,
    ) -> Result<CopySession, RemoteError> {
        super::pack_id(id)?;
        if let Some(source) = source {
            if crate::server::lan::validate_bind(source).is_err()
                || source.is_ipv6() != self.uses_ipv6()
            {
                return Err(RemoteError::InvalidInput);
            }
        }
        if self.closed.is_cancelled() {
            return Err(RemoteError::Closed);
        }
        Ok(CopySession {
            id: id.to_owned(),
            library: RemoteLibrary {
                // The view's pool may contain sockets owned by its departing runtime.
                client: super::pinned_client(&self.certificate, source)?,
                source_address: source,
                certificate: self.certificate.clone(),
                origin: self.origin.clone(),
                api: self.api.clone(),
                content: self.content.clone(),
                csrf: self.csrf.clone(),
                closed: tokio_util::sync::CancellationToken::new(),
            },
        })
    }
    pub async fn copy_pack(
        &self,
        id: &str,
        root: PathBuf,
        action: impl Fn() -> DownloadAction + Send + 'static,
        progress: impl FnMut(DownloadProgress) -> Result<(), PackError> + Send + 'static,
    ) -> Result<CopyOutcome, CopyError> {
        self.copy_pack_observed(id, root, action, progress, |_, _| Ok(()))
            .await
    }
    pub(crate) async fn copy_pack_observed(
        &self,
        id: &str,
        root: PathBuf,
        action: impl Fn() -> DownloadAction + Send + 'static,
        progress: impl FnMut(DownloadProgress) -> Result<(), PackError> + Send + 'static,
        prepared: impl FnOnce(&crate::sync::CopyLease<'_>, &PackManifest) -> Result<(), PackError>
            + Send
            + 'static,
    ) -> Result<CopyOutcome, CopyError> {
        super::pack_id(id)?;
        let manifest: PackManifest =
            serde_json::from_value(self.json(&format!("/api/pack-transfer/{id}"), &[]).await?)
                .map_err(|_| RemoteError::InvalidResponse)?;
        manifest.validate()?;
        if manifest.id != id {
            return Err(RemoteError::InvalidResponse.into());
        }
        let url = self
            .origin
            .join(&format!("transfer/{id}/pack"))
            .map_err(|_| RemoteError::InvalidInput)?;
        if url.origin() != self.origin.origin() {
            return Err(RemoteError::InvalidInput.into());
        }
        let certificate = self.certificate.clone();
        let source_address = self.source_address;
        let cookie = self.content.clone();
        let closed = self.closed.clone();
        let cancelled = tokio_util::sync::CancellationToken::new();
        let _cancel_on_drop = cancelled.clone().drop_guard();
        tokio::task::spawn_blocking(move || {
            let client = blocking_client(&certificate, source_address, &cookie)?;
            let store = SyncStore::open(&root)?;
            let lease = store.copy_lease()?;
            prepared(&lease, &manifest)?;
            let mut spec = store.cache_spec(&manifest)?;
            spec.url = Some(url.to_string());
            let admitted = || {
                !closed.is_cancelled()
                    && !cancelled.is_cancelled()
                    && action() == DownloadAction::Continue
            };
            let outcome = crate::download::fetch_pack_controlled_with_client(
                &spec,
                &root,
                &client,
                || {
                    if closed.is_cancelled() || cancelled.is_cancelled() {
                        DownloadAction::Cancel
                    } else {
                        action()
                    }
                },
                progress,
            )?;
            match outcome {
                DownloadOutcome::Paused => Ok(CopyOutcome::Paused),
                DownloadOutcome::Cancelled => {
                    lease.discard(&manifest)?;
                    Ok(CopyOutcome::Cancelled)
                }
                DownloadOutcome::Complete(_) => {
                    match store.install_cached_if(&manifest, admitted)? {
                        Some(row) => Ok(CopyOutcome::Complete(row)),
                        None => {
                            lease.discard(&manifest)?;
                            Ok(CopyOutcome::Cancelled)
                        }
                    }
                }
            }
        })
        .await
        .map_err(|_| RemoteError::Unavailable)?
    }
}

fn blocking_client(
    certificate: &[u8],
    source_address: Option<std::net::IpAddr>,
    cookie: &str,
) -> Result<reqwest::blocking::Client, RemoteError> {
    let builder = reqwest::Client::builder()
        .local_address(source_address)
        .no_proxy()
        .referer(false)
        .redirect(reqwest::redirect::Policy::none())
        .tls_built_in_root_certs(false)
        .add_root_certificate(
            reqwest::Certificate::from_der(certificate).map_err(|_| RemoteError::InvalidHost)?,
        )
        .default_headers({
            let mut headers = reqwest::header::HeaderMap::new();
            let mut value = reqwest::header::HeaderValue::from_str(cookie)
                .map_err(|_| RemoteError::InvalidInput)?;
            value.set_sensitive(true);
            headers.insert(reqwest::header::COOKIE, value);
            headers
        })
        .connect_timeout(std::time::Duration::from_secs(10))
        .read_timeout(std::time::Duration::from_secs(30));
    reqwest::blocking::ClientBuilder::from(builder)
        .timeout(None)
        .build()
        .map_err(|_| RemoteError::Unavailable)
}
