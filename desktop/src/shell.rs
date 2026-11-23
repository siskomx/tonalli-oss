use std::sync::atomic::Ordering;
use std::sync::Arc;

use tauri::menu::{Menu, MenuItem, Submenu};
use tauri::tray::TrayIconBuilder;
use tauri::webview::NewWindowResponse;
use tauri::{AppHandle, Manager, RunEvent, WebviewUrl, WebviewWindowBuilder};
use tauri_plugin_dialog::DialogExt;
use tonalli_desktop::{navigation, options::Options};

use crate::session::Session;

const NAME: &str = env!("TONALLI_DISPLAY_NAME");

fn media_export_dialog(app:&AppHandle,session:&Arc<Session>,root:std::path::PathBuf,web:std::path::PathBuf,id:String) {
    if session.content_busy.swap(true,Ordering::AcqRel){return;}
    let owner=Arc::clone(session);let app=app.clone();let fallback=app.clone();
    let started=std::thread::Builder::new().name("desktop-media-save".into()).spawn(move|| {
        let metadata=tonalli_core::media::MediaStore::open(&root).and_then(|store|store.get(&id));
        let Ok(metadata)=metadata else {owner.content_busy.store(false,Ordering::Release);message(&app,"This managed file is unavailable. Your original files were preserved.");return;};
        if owner.closing.load(Ordering::Acquire){owner.content_busy.store(false,Ordering::Release);return;}
        let dispatcher=app.clone();let recovery=Arc::clone(&owner);
        if dispatcher.run_on_main_thread(move|| {
            let dialog_app=app.clone();let recovery=Arc::clone(&owner);
            app.dialog().file().set_title("Save a copy").set_file_name(&metadata.name).save_file(move|selected| {
                let Some(selected)=selected else {owner.content_busy.store(false,Ordering::Release);return;};
                let worker_owner=Arc::clone(&owner);let worker_app=dialog_app.clone();
                let started=std::thread::Builder::new().name("desktop-media-export".into()).spawn(move|| {
                    let result=(||->Result<(),Box<dyn std::error::Error>> {
                        let destination=selected.into_path().map_err(|_|"Choose a local destination")?;
                        tonalli_desktop::media::export_copy(&root,&web,&id,&destination)
                    })();
                    worker_owner.content_busy.store(false,Ordering::Release);
                    if !worker_owner.closing.load(Ordering::Acquire){message(&worker_app,if result.is_ok(){"Copy saved."}else{"The copy could not be saved. Your managed file and any existing destination were preserved; check storage and try again."});}
                });
                if started.is_err(){recovery.content_busy.store(false,Ordering::Release);message(&dialog_app,"The save worker could not start.");}
            });
        }).is_err(){recovery.content_busy.store(false,Ordering::Release);}
    });
    if started.is_err(){session.content_busy.store(false,Ordering::Release);message(&fallback,"The save worker could not start.");}
}

fn map_dialog(app: &AppHandle, session: &Arc<Session>, port: u16) {
    if session.content_busy.swap(true, Ordering::AcqRel) { return; }
    let owner = Arc::clone(session);
    let app = app.clone();
    app.clone().dialog().file().set_title("Choose map file").add_filter("PMTiles map", &["pmtiles"]).pick_file(move |selected| {
        let Some(selected) = selected else {owner.content_busy.store(false, Ordering::Release); return};
        let result_owner = Arc::clone(&owner);
        let result_app = app.clone();
        let started = std::thread::Builder::new().name("desktop-map-import".into()).spawn(move || {
            let result = selected.into_path().map_err(|_| ()).and_then(|path| result_owner.import_map(&path).map_err(|_| ()));
            result_owner.content_busy.store(false, Ordering::Release);
            if result_owner.closing.load(Ordering::Acquire) {return;}
            if result.is_ok() {
                if let Some(window) = result_app.get_webview_window("main") { if let Ok(url) = format!("http://127.0.0.1:{port}/map").parse() {let _ = window.navigate(url);} }
            } else {message(&result_app, "Map could not be imported. Choose a complete PMTiles file and check free storage. Existing maps and your original file were preserved.");}
        });
        if started.is_err() {owner.content_busy.store(false, Ordering::Release);message(&app,"Map import could not start.");}
    });
}

fn message(app: &AppHandle, text: impl Into<String>) {
    app.dialog().message(text).title(NAME).show(|_| {});
}

/// "Restart and update": the package is verified and unpacked here, then a script swaps the files
/// after this process exits and starts the application again.
fn apply_update(app: &AppHandle, session: &Arc<Session>, data: std::path::PathBuf) {
    if session.content_busy.swap(true, Ordering::AcqRel) {
        message(app, "Another operation is in progress. Try the update again when it finishes.");
        return;
    }
    let owner = Arc::clone(session);
    let app = app.clone();
    let started = std::thread::Builder::new().name("desktop-update".into()).spawn(move || {
        let result = (|| -> Result<(), String> {
            let staged = tonalli_desktop::update::stage(&data)?;
            let exe = std::env::current_exe().map_err(|error| error.to_string())?;
            let install = exe.parent().ok_or("The application folder is unavailable.")?;
            let work = data.join("updates");
            let args: Vec<String> = std::env::args().skip(1).collect();
            tonalli_desktop::update::spawn_apply(&tonalli_desktop::update::Apply {
                staged: &staged,
                install,
                exe: &exe,
                args: &args,
                wait_pid: std::process::id(),
                work: &work,
            })
            .map(|_| ())
            .map_err(|error| error.to_string())
        })();
        match result {
            // The shutdown hook runs on exit; the script waits for this process before touching files.
            Ok(()) => app.exit(0),
            Err(_) => {
                owner.content_busy.store(false, Ordering::Release);
                message(&app, "The update could not be installed. Nothing was changed; the downloaded file is checked again before the next try.");
            }
        }
    });
    if started.is_err() {
        session.content_busy.store(false, Ordering::Release);
    }
}

fn content_dialog(app: &AppHandle, session: &Arc<Session>, port: u16) {
    if session
        .content_busy
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        message(
            app,
            "Content is already being opened. Your library remains available.",
        );
        return;
    }
    let owner = Arc::clone(session);
    let app_handle = app.clone();
    app.dialog().file().set_title("Open a file")
        .pick_files(move |selected| {
            let Some(selected) = selected else { owner.content_busy.store(false, Ordering::Release); return; };
            let result_owner = Arc::clone(&owner);
            let result_app = app_handle.clone();
            let started = std::thread::Builder::new().name("desktop-content-open".into()).spawn(move || {
                let result = selected.into_iter().map(|file| file.into_path().map_err(|_| "The picker did not return local files".to_owned()))
                    .collect::<Result<Vec<_>, _>>()
                    .and_then(|paths| result_owner.open_content(paths).map_err(|error| error.to_string()));
                result_owner.content_busy.store(false, Ordering::Release);
                if result_owner.closing.load(Ordering::Acquire) { return; }
                match result {
                    Ok(id) => {
                        if let Some(window) = result_app.get_webview_window("main") {
                            if let Ok(mut url) = format!("http://127.0.0.1:{port}/library").parse::<url::Url>() {
                                url.query_pairs_mut().append_pair("pack", &id);
                                let _ = window.navigate(url);
                            }
                        }
                    }
                    Err(_) => message(&result_app, "Content could not be opened. Check that every split part is selected. Your existing sources and files were preserved."),
                }
            });
            if started.is_err() {
                owner.content_busy.store(false, Ordering::Release);
                message(&app_handle, "The content worker could not start. Your library was preserved.");
            }
        });
}

fn iso_dialog(app: &AppHandle, session: &Arc<Session>, port: u16) {
    if session.iso_inner.is_none() {
        message(
            app,
            "The M1 ISO probe needs an explicit --iso-inner /PATH.zim startup argument.",
        );
        return;
    }
    if session
        .iso_busy
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        message(app, "An ISO selection or open is already in progress.");
        return;
    }
    let owner = Arc::clone(session);
    let app_handle = app.clone();
    app.dialog()
        .file()
        .set_title("Open ISO read-only")
        .add_filter("Optical image", &["iso", "udf"])
        .pick_file(move |selected| {
            let Some(selected) = selected else {
                owner.iso_busy.store(false, Ordering::Release);
                return;
            };
            let result_owner = Arc::clone(&owner);
            let result_app = app_handle.clone();
            let started = std::thread::Builder::new()
                .name("desktop-iso-open".into())
                .spawn(move || {
                    let result = selected
                        .into_path()
                        .map_err(|_| "The picker did not return a local file".to_owned())
                        .and_then(|path| {
                            result_owner
                                .open_iso(&path)
                                .map_err(|error| error.to_string())
                        });
                    result_owner.iso_busy.store(false, Ordering::Release);
                    if result_owner.closing.load(Ordering::Acquire) {
                        return;
                    }
                    match result {
                        Ok(()) => {
                            if let Some(window) = result_app.get_webview_window("main") {
                                if let Ok(url) =
                                    format!("http://127.0.0.1:{port}/library?pack=iso").parse()
                                {
                                    let _ = window.navigate(url);
                                    let _ = window.show();
                                    let _ = window.set_focus();
                                }
                            }
                        }
                        Err(error) => message(
                            &result_app,
                            format!(
                                "ISO open failed; existing packs were preserved.\n{}",
                                error.chars().take(512).collect::<String>()
                            ),
                        ),
                    }
                });
            if started.is_err() {
                owner.iso_busy.store(false, Ordering::Release);
                message(
                    &app_handle,
                    "The ISO worker could not start; existing packs were preserved.",
                );
            }
        });
}

fn folder_dialog(app: &AppHandle, session: &Arc<Session>, port: u16) {
    if session
        .content_busy
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        message(
            app,
            "Content is already being opened. Your library remains available.",
        );
        return;
    }
    let owner = Arc::clone(session);
    let app_handle = app.clone();
    app.dialog().file().set_title("Open folder").pick_folder(move |selected| {
        let Some(selected) = selected else { owner.content_busy.store(false, Ordering::Release); return; };
        let result_owner = Arc::clone(&owner);
        let result_app = app_handle.clone();
        let started = std::thread::Builder::new().name("desktop-folder-open".into()).spawn(move || {
            let result = selected.into_path().map_err(|_| "The picker did not return a local folder".to_owned())
                .and_then(|path| result_owner.open_folder(&path).map_err(|_| "Folder could not be scanned".to_owned()));
            result_owner.content_busy.store(false, Ordering::Release);
            if result_owner.closing.load(Ordering::Acquire) { return; }
            match result {
                Ok(summary) => {
                    if let Some(id) = summary.first_id {
                        if let Some(window) = result_app.get_webview_window("main") {
                            if let Ok(mut url) = format!("http://127.0.0.1:{port}/library").parse::<url::Url>() {
                                url.query_pairs_mut().append_pair("pack", &id);
                                let _ = window.navigate(url);
                            }
                        }
                    }
                    message(&result_app, format!("Opened {} books; {} could not be opened; {} other items skipped. Your existing books and files were preserved.", summary.opened, summary.failed, summary.ignored));
                }
                Err(_) => message(&result_app, "Folder could not be opened. Choose a smaller, readable folder. Your existing books and files were preserved."),
            }
        });
        if started.is_err() {
            owner.content_busy.store(false, Ordering::Release);
            message(&app_handle, "The content worker could not start. Your library was preserved.");
        }
    });
}

pub fn run(
    options: Options,
    started: std::time::Instant,
) -> Result<(), Box<dyn std::error::Error>> {
    let (session, info) = Session::start(&options)?;
    let port = info.port;
    let setup_session = Arc::clone(&session);
    let menu_session = Arc::clone(&session);
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .on_menu_event(move |app, event| match event.id.as_ref() {
            "open-content" => content_dialog(app, &menu_session, port),
            "open-folder" => folder_dialog(app, &menu_session, port),
            "open-iso" => iso_dialog(app, &menu_session, port),
            "show" => {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.show();
                    let _ = window.set_focus();
                }
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .setup(move |app| {
            let open_content = MenuItem::with_id(app, "open-content", "Open a file…", true, Some("Ctrl+O"))?;
            let open_folder = MenuItem::with_id(app, "open-folder", "Open folder…", true, None::<&str>)?;
            let open = MenuItem::with_id(app, "open-iso", "Open ISO (configured path)…", true, None::<&str>)?;
            let show = MenuItem::with_id(app, "show", "Show window", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "Quit", true, Some("Ctrl+Q"))?;
            let file = Submenu::with_items(app, "File", true, &[&open_content, &open_folder, &show, &quit])?;
            let advanced = Submenu::with_items(app, "Advanced", true, &[&open])?;
            let settings = Submenu::with_items(app, "Settings", true, &[&advanced])?;
            let menu = Menu::with_items(app, &[&file, &settings])?;
            let icon = app
                .default_window_icon()
                .cloned()
                .ok_or("Missing compiled application icon")?;
            TrayIconBuilder::with_id("main")
                .icon(icon)
                .tooltip(NAME)
                .menu(&menu)
                .build(app)?;
            let action_app = app.handle().clone();
            let action_session = Arc::clone(&setup_session);
            let media_root = options.data.clone();
            let media_web = options.web.clone();
            WebviewWindowBuilder::new(app, "main", WebviewUrl::External(info.launch_url.parse()?))
                .title(NAME)
                .inner_size(1100.0, 800.0)
                .user_agent("Mozilla/5.0 TonalliNative/Windows")
                .menu(menu)
                .data_directory(options.profile.canonicalize()?)
                .on_navigation(move |url| {
                    if let Some(action) = navigation::native_action(port, url) {
                        let app = action_app.clone();
                        let session = Arc::clone(&action_session);
                        let open = action == "open";
                        let map = action == "open-map";
                        let update = action == "apply-update";
                        let update_data = media_root.clone();
                        let export = navigation::media_export_id(url.path()).map(str::to_owned);
                        let root = media_root.clone();
                        let web = media_web.clone();
                        let dispatcher = app.clone();
                        let _ = dispatcher.run_on_main_thread(move || {
                            if let Some(id)=export {media_export_dialog(&app,&session,root,web,id);}
                            else if open { content_dialog(&app, &session, port); }
                            else if map { map_dialog(&app, &session, port); }
                            else if update { apply_update(&app, &session, update_data); }
                            else { iso_dialog(&app, &session, port); }
                        });
                        return false;
                    }
                    navigation::allows(port, url)
                })
                .on_new_window(|_, _| NewWindowResponse::Deny)
                .build()?;
            // Native window construction only; first paint/content readiness are measured separately.
            eprintln!(
                "desktop_window_created_ms={:.3}",
                started.elapsed().as_secs_f64() * 1000.0
            );
            let unavailable_sources = setup_session.unavailable_sources;
            app.manage(setup_session);
            if unavailable_sources > 0 {
                message(app.handle(), "Some saved books are unavailable. Your files were preserved; use File → Open a file to select them again.");
            }
            Ok(())
        })
        .build(tauri::generate_context!())?;
    app.run(move |_, event| {
        if matches!(event, RunEvent::Exit) {
            let _ = session.shutdown();
        }
    });
    Ok(())
}
