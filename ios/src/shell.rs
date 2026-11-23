use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::Arc;

use tauri::{AppHandle, Manager, RunEvent, WebviewUrl, WebviewWindowBuilder};
use tauri_plugin_dialog::{DialogExt, FilePath};

use crate::options::Options;
use crate::session::Session;
use crate::{navigation, web};

const NAME: &str = "Tonalli";

fn message(app: &AppHandle, text: impl Into<String>) {
    app.dialog().message(text).title(NAME).show(|_| {});
}

/// Copies picked files into the app's Documents folder, which is also where Files-app drops land.
fn import(selected: Vec<FilePath>, documents: &Path) -> Result<(), String> {
    std::fs::create_dir_all(documents).map_err(|error| error.to_string())?;
    for file in selected {
        let source = file
            .into_path()
            .map_err(|_| "The picker did not return a local file".to_owned())?;
        let name = source.file_name().ok_or("The picked file has no name")?;
        let destination = documents.join(name);
        if destination == source {
            continue;
        }
        let same = std::fs::metadata(&destination)
            .and_then(|existing| std::fs::metadata(&source).map(|picked| existing.len() == picked.len()))
            .unwrap_or(false);
        if same {
            continue;
        }
        let mut partial = destination.clone().into_os_string();
        partial.push(".part");
        let partial = PathBuf::from(partial);
        std::fs::copy(&source, &partial).map_err(|error| error.to_string())?;
        std::fs::rename(&partial, &destination).map_err(|error| error.to_string())?;
    }
    Ok(())
}

/// "Save a copy": iPhone has no save dialog here, so the copy goes to Documents/Exports, which the Files app shows.
fn export_copy(
    app: &AppHandle,
    session: &Arc<Session>,
    root: PathBuf,
    web: PathBuf,
    documents: PathBuf,
    id: String,
) {
    if session
        .content_busy
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        message(app, "Another file operation is still running. Try again in a moment.");
        return;
    }
    let owner = Arc::clone(session);
    let app_handle = app.clone();
    let started = std::thread::Builder::new()
        .name("ios-media-export".into())
        .spawn(move || {
            let result = (|| -> Result<(), Box<dyn std::error::Error>> {
                let metadata = tonalli_core::media::MediaStore::open(&root)?.get(&id)?;
                let destination =
                    tonalli_desktop::media::exports_destination(&documents, &metadata.name)?;
                std::fs::create_dir_all(destination.parent().ok_or("Exports folder unavailable")?)?;
                tonalli_desktop::media::export_copy(&root, &web, &id, &destination)
            })();
            owner.content_busy.store(false, Ordering::Release);
            if owner.closing.load(Ordering::Acquire) {
                return;
            }
            message(
                &app_handle,
                if result.is_ok() {
                    "Saved. Open the Files app, then Tonalli, then Exports."
                } else {
                    "The copy could not be saved. Your file was not changed; check free storage and try again."
                },
            );
        });
    if started.is_err() {
        session.content_busy.store(false, Ordering::Release);
        message(app, "The save worker could not start.");
    }
}

/// "Choose map file": the same Core import as Windows. The picker hands back a copy in app storage.
fn map_dialog(app: &AppHandle, session: &Arc<Session>, port: u16) {
    if session
        .content_busy
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        message(app, "Content is already being opened. Your library remains available.");
        return;
    }
    let owner = Arc::clone(session);
    let app_handle = app.clone();
    app.dialog().file().set_title("Choose map file").pick_file(move |selected| {
        let Some(selected) = selected else {
            owner.content_busy.store(false, Ordering::Release);
            return;
        };
        let result_owner = Arc::clone(&owner);
        let result_app = app_handle.clone();
        let started = std::thread::Builder::new()
            .name("ios-map-import".into())
            .spawn(move || {
                let result = selected
                    .into_path()
                    .map_err(|_| ())
                    .and_then(|path| result_owner.import_map(&path).map_err(|_| ()));
                result_owner.content_busy.store(false, Ordering::Release);
                if result_owner.closing.load(Ordering::Acquire) {
                    return;
                }
                if result.is_ok() {
                    if let Some(window) = result_app.get_webview_window("main") {
                        if let Ok(url) = format!("http://127.0.0.1:{port}/map").parse::<url::Url>() {
                            let _ = window.navigate(url);
                        }
                    }
                } else {
                    message(
                        &result_app,
                        "Map could not be imported. Choose a complete PMTiles file and check free storage. Existing maps and your original file were preserved.",
                    );
                }
            });
        if started.is_err() {
            owner.content_busy.store(false, Ordering::Release);
            message(&app_handle, "Map import could not start.");
        }
    });
}

fn open_dialog(app: &AppHandle, session: &Arc<Session>, port: u16, documents: PathBuf) {
    if session
        .content_busy
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        message(app, "Content is already being opened. Your library remains available.");
        return;
    }
    let owner = Arc::clone(session);
    let app_handle = app.clone();
    app.dialog().file().set_title("Open a file").pick_files(move |selected| {
        let Some(selected) = selected else {
            owner.content_busy.store(false, Ordering::Release);
            return;
        };
        let result_owner = Arc::clone(&owner);
        let result_app = app_handle.clone();
        let started = std::thread::Builder::new()
            .name("ios-content-open".into())
            .spawn(move || {
                let result = import(selected, &documents).and_then(|()| {
                    result_owner
                        .open_folder(&documents)
                        .map_err(|error| error.to_string())
                });
                result_owner.content_busy.store(false, Ordering::Release);
                if result_owner.closing.load(Ordering::Acquire) {
                    return;
                }
                match result {
                    Ok(summary) => {
                        if let Some(id) = summary.first_id {
                            if let Some(window) = result_app.get_webview_window("main") {
                                if let Ok(mut url) =
                                    format!("http://127.0.0.1:{port}/library").parse::<url::Url>()
                                {
                                    url.query_pairs_mut().append_pair("pack", &id);
                                    let _ = window.navigate(url);
                                }
                            }
                        }
                    }
                    Err(_) => message(
                        &result_app,
                        "Content could not be opened. Check free storage and that the file is complete. Your existing files were preserved.",
                    ),
                }
            });
        if started.is_err() {
            owner.content_busy.store(false, Ordering::Release);
            message(&app_handle, "The content worker could not start. Your library was preserved.");
        }
    });
}

pub fn run() {
    let built = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let data = app.path().app_data_dir()?;
            let documents = app.path().document_dir()?;
            std::fs::create_dir_all(&data)?;
            let web_dir = web::install(&data)?;
            let export_root = data.clone();
            let export_web = web_dir.clone();
            let options = Options {
                headless: None,
                web: web_dir,
                profile: data.join("webview"),
                data,
                packs: Vec::new(),
                title_indexes: HashMap::new(),
                iso_inner: None,
                ai_model: None,
            };
            let (session, info) = Session::start(&options)?;
            let port = info.port;
            if documents.is_dir() {
                let _ = session.open_folder(&documents);
            }
            let action_app = app.handle().clone();
            let action_session = Arc::clone(&session);
            WebviewWindowBuilder::new(app, "main", WebviewUrl::External(info.launch_url.parse()?))
                .user_agent("Mozilla/5.0 TonalliNative/iOS")
                .on_navigation(move |url| {
                    if let Some(action) = navigation::native_action(port, url) {
                        let app = action_app.clone();
                        let session = Arc::clone(&action_session);
                        let documents = documents.clone();
                        let dispatcher = app.clone();
                        let action = action.to_owned();
                        let export = navigation::media_export_id(url.path()).map(str::to_owned);
                        let (root, web) = (export_root.clone(), export_web.clone());
                        let _ = dispatcher.run_on_main_thread(move || {
                            match (action.as_str(), export) {
                                ("open", _) => open_dialog(&app, &session, port, documents),
                                ("open-map", _) => map_dialog(&app, &session, port),
                                ("export-media", Some(id)) => {
                                    export_copy(&app, &session, root, web, documents, id)
                                }
                                _ => message(&app, "This is not available on iPhone yet."),
                            }
                        });
                        return false;
                    }
                    navigation::allows(port, url)
                })
                .build()?;
            app.manage(session);
            Ok(())
        })
        .build(tauri::generate_context!());
    match built {
        Ok(app) => app.run(|handle, event| {
            if matches!(event, RunEvent::Exit) {
                if let Some(session) = handle.try_state::<Arc<Session>>() {
                    let _ = session.shutdown();
                }
            }
        }),
        Err(error) => eprintln!("Tonalli could not start: {error}"),
    }
}
