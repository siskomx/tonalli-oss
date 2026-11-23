#![cfg(feature = "ffi")]
use reqwest::{
    blocking::Client,
    header::{COOKIE, SET_COOKIE},
};
use serde_json::{json, Value};
use std::{
    fs,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    },
};
use tonalli_core::{
    ffi::*,
    media::{MediaFile, MediaStore},
    pack::{PackError, PreadSource},
    server::{SharedMedia, TvCatalog, TvStatus},
};
const WAV: &[u8] =
    b"RIFF\x24\0\0\0WAVEfmt \x10\0\0\0\x01\0\x01\0\x40\x1f\0\0\x80\x3e\0\0\x02\0\x10\0data\0\0\0\0";
fn fixture() -> (tempfile::TempDir, Arc<TvCatalog>, MediaFile) {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("source.wav");
    fs::write(&path, WAV).unwrap();
    fs::create_dir(root.path().join("data")).unwrap();
    let store = MediaStore::open(&root.path().join("data")).unwrap();
    let file = store
        .import(
            Arc::new(PreadSource::open(&path).unwrap()),
            "Selected.wav",
            "",
        )
        .unwrap();
    let catalog = Arc::new(
        TvCatalog::new(Arc::new(
            SharedMedia::capture(&root.path().join("data"), std::slice::from_ref(&file.id))
                .unwrap(),
        ))
        .unwrap(),
    );
    (root, catalog, file)
}
#[test]
fn tv_ffi_owns_selected_catalog_and_aliases_stop_idempotently() {
    let (_root, catalog, file) = fixture();
    let server = server_start_tv(catalog, "127.0.0.1".into(), 0).unwrap();
    let alias = server.clone();
    let TvInfo { address, port, url } = tv_info(server.clone()).unwrap();
    assert_eq!(address, "127.0.0.1");
    assert_ne!(port, 0);
    assert_eq!(url, format!("http://127.0.0.1:{port}/description.xml"));
    let client = Client::builder().no_proxy().build().unwrap();
    let bytes = client
        .get(format!("http://{address}:{port}/media/{}", file.id))
        .send()
        .unwrap()
        .bytes()
        .unwrap();
    assert_eq!(bytes.as_ref(), WAV);
    server_stop_tv(server).unwrap();
    assert!(matches!(
        tv_info(alias.clone()),
        Err(PackError::NotFound(_))
    ));
    server_stop_tv(alias).unwrap();
}
#[test]
fn tv_ffi_rejects_non_literal_public_wildcard_and_non_ipv4_bindings() {
    let (_root, catalog, _) = fixture();
    for bind in ["0.0.0.0", "8.8.8.8", "example.invalid", "::1"] {
        assert!(
            matches!(
                server_start_tv(catalog.clone(), bind.into(), 0),
                Err(PackError::Mismatch(_))
            ),
            "{bind}"
        );
    }
}
#[derive(Default)]
struct Scheduler {
    server: Mutex<Option<Arc<TvHandle>>>,
    starts: AtomicUsize,
    stops: AtomicUsize,
    detach_on_close: bool,
    selection: Vec<MediaFile>,
}
impl TvScheduler for Scheduler {
    fn status(&self) -> Result<TvStatus, PackError> {
        Ok(TvStatus {
            running: self.server.lock().unwrap().is_some(),
            selected_media: self.selection.clone(),
            ..TvStatus::default()
        })
    }
    fn start(&self, network: String, catalog: Arc<TvCatalog>) -> Result<(), PackError> {
        assert_eq!(network, "owned-network");
        self.starts.fetch_add(1, Ordering::SeqCst);
        *self.server.lock().unwrap() = Some(server_start_tv(catalog, "127.0.0.1".into(), 0)?);
        Ok(())
    }
    fn stop(&self) -> Result<(), PackError> {
        self.stops.fetch_add(1, Ordering::SeqCst);
        if let Some(server) = self.server.lock().unwrap().take() {
            server_stop_tv(server)?;
        }
        Ok(())
    }
    fn detach_view(&self) -> Result<(), PackError> {
        if self.detach_on_close {
            Ok(())
        } else {
            self.stop()
        }
    }
}
#[test]
fn tv_foreground_owner_survives_private_view_closure_without_retaining_root() {
    let (root, catalog, file) = fixture();
    let web = root.path().join("web");
    fs::create_dir(&web).unwrap();
    fs::write(web.join("index.html"), "fixture").unwrap();
    let source = server_start_loopback(
        root.path().join("data").to_string_lossy().into_owned(),
        web.to_string_lossy().into_owned(),
    )
    .unwrap();
    let weak = Arc::downgrade(&source);
    let scheduler = Arc::new(Scheduler {
        detach_on_close: true,
        selection: vec![file.clone()],
        ..Scheduler::default()
    });
    assert_eq!(catalog.selected_media()[0].sha256, file.sha256);
    scheduler.start("owned-network".into(), catalog).unwrap();
    let native = scheduler.server.lock().unwrap().as_ref().unwrap().clone();
    server_attach_tv(source.clone(), scheduler.clone()).unwrap();
    server_stop_loopback(source.clone()).unwrap();
    drop(source);
    assert!(
        weak.upgrade().is_none(),
        "foreground owner must not retain the private root"
    );
    let info = tv_info(native.clone())
        .expect("private view closure must leave the explicit foreground owner running");
    let client = Client::builder().no_proxy().build().unwrap();
    let response = client
        .get(format!(
            "http://{}:{}/media/{}",
            info.address, info.port, file.id
        ))
        .send()
        .unwrap();
    assert_eq!(response.bytes().unwrap().as_ref(), WAV);
    let reopened = server_start_loopback(
        root.path().join("data").to_string_lossy().into_owned(),
        web.to_string_lossy().into_owned(),
    )
    .unwrap();
    server_attach_tv(reopened.clone(), scheduler.clone()).unwrap();
    let state = private_tv_status(&reopened);
    assert_eq!(state["status"]["running"], true);
    assert_eq!(
        state["selected_media"][0]["id"], file.id,
        "a recreated private view must recover the owner's exact selected snapshot"
    );
    server_stop_loopback(reopened).unwrap();
    scheduler.stop().unwrap();
    assert!(tv_info(native).is_err());
}
fn private_tv_status(source: &Arc<LoopbackHandle>) -> Value {
    let info = server_info(source.clone()).unwrap();
    let client = Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();
    let launch = client.get(info.launch_url).send().unwrap();
    let cookie = launch
        .headers()
        .get_all(SET_COOKIE)
        .iter()
        .map(|value| value.to_str().unwrap().split(';').next().unwrap())
        .collect::<Vec<_>>()
        .join("; ");
    let base = format!("http://127.0.0.1:{}", info.port);
    let session: Value = client
        .get(format!("{base}/api/session"))
        .header(COOKIE, &cookie)
        .header("Sec-Fetch-Dest", "empty")
        .send()
        .unwrap()
        .json()
        .unwrap();
    client
        .get(format!("{base}/api/tv"))
        .header(COOKIE, &cookie)
        .header("Sec-Fetch-Dest", "empty")
        .header("X-Tonalli-Csrf", session["csrf"].as_str().unwrap())
        .send()
        .unwrap()
        .json()
        .unwrap()
}
#[test]
fn tv_scheduler_is_private_closes_once_and_does_not_retain_root_handle() {
    let (root, _catalog, file) = fixture();
    let web = root.path().join("web");
    fs::create_dir(&web).unwrap();
    fs::write(web.join("index.html"), "fixture").unwrap();
    let source = server_start_loopback(
        root.path().join("data").to_string_lossy().into_owned(),
        web.to_string_lossy().into_owned(),
    )
    .unwrap();
    let weak = Arc::downgrade(&source);
    let scheduler = Arc::new(Scheduler::default());
    server_attach_tv(source.clone(), scheduler.clone()).unwrap();
    let info = server_info(source.clone()).unwrap();
    let client = Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();
    let launch = client.get(info.launch_url).send().unwrap();
    let cookie = launch
        .headers()
        .get_all(SET_COOKIE)
        .iter()
        .map(|v| v.to_str().unwrap().split(';').next().unwrap())
        .collect::<Vec<_>>()
        .join("; ");
    let base = format!("http://127.0.0.1:{}", info.port);
    let session: Value = client
        .get(format!("{base}/api/session"))
        .header(COOKIE, &cookie)
        .header("Sec-Fetch-Dest", "empty")
        .send()
        .unwrap()
        .json()
        .unwrap();
    assert_eq!(session["private_tv"], true);
    let start = client
        .post(format!("{base}/api/tv/start"))
        .header(COOKIE, &cookie)
        .header("Sec-Fetch-Dest", "empty")
        .header("X-Tonalli-Csrf", session["csrf"].as_str().unwrap())
        .json(&json!({"network":"owned-network","media":[file.id]}))
        .send()
        .unwrap();
    assert_eq!(start.status(), 200);
    assert_eq!(scheduler.starts.load(Ordering::SeqCst), 1);
    let native = scheduler.server.lock().unwrap().as_ref().unwrap().clone();
    assert!(tv_info(native.clone()).is_ok());
    server_stop_loopback(source.clone()).unwrap();
    server_stop_loopback(source.clone()).unwrap();
    assert_eq!(scheduler.stops.load(Ordering::SeqCst), 1);
    assert!(matches!(tv_info(native), Err(PackError::NotFound(_))));
    assert!(server_attach_tv(source.clone(), scheduler).is_err());
    drop(source);
    assert!(
        weak.upgrade().is_none(),
        "catalog dispatcher must not retain private root"
    );
}
