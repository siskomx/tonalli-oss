//! iPhone shell. It compiles the Windows shell's portable modules unchanged, by path,
//! around the same Core and the same web UI; only the window and file handling differ.
extern crate self as tonalli_desktop;

#[path = "../../desktop/src/content.rs"]
pub mod content;
#[path = "../../desktop/src/downloads.rs"]
pub mod downloads;
#[path = "../../desktop/src/maps.rs"]
pub mod maps;
#[path = "../../desktop/src/media.rs"]
pub mod media;
#[path = "../../desktop/src/navigation.rs"]
pub mod navigation;
#[path = "../../desktop/src/options.rs"]
pub mod options;
#[path = "../../desktop/src/preparation.rs"]
pub mod preparation;
#[path = "../../desktop/src/sharing.rs"]
pub mod sharing;
#[path = "../../desktop/src/session.rs"]
mod session;

mod shell;
mod web;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    shell::run();
}
