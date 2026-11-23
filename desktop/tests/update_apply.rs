#![cfg(all(windows, feature = "shell"))]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use tonalli_desktop::update::{spawn_apply, Apply};

struct Fixture {
    temp: tempfile::TempDir,
}

impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        for folder in ["install", "install/web", "install/other", "staged", "staged/web", "work"] {
            fs::create_dir_all(root.join(folder)).unwrap();
        }
        fs::write(root.join("install/a.txt"), "old a").unwrap();
        fs::write(root.join("install/keep.txt"), "keep me").unwrap();
        fs::write(root.join("install/web/index.html"), "old page").unwrap();
        fs::write(root.join("install/web/old-5f3a.js"), "from the previous version").unwrap();
        fs::write(root.join("install/other/notes.txt"), "not part of the package").unwrap();
        // Stands in for the application: relaunching it leaves a marker, holding the arguments it was
        // started with, next to the install folder.
        fs::write(root.join("install/app.cmd"), "@echo launched %*> \"%~dp0..\\marker.txt\"\r\n").unwrap();
        fs::write(root.join("staged/a.txt"), "new a").unwrap();
        fs::write(root.join("staged/b.txt"), "new b").unwrap();
        fs::write(root.join("staged/web/index.html"), "new page").unwrap();
        fs::write(root.join("staged/app.cmd"), "@echo launched %*> \"%~dp0..\\marker.txt\"\r\n").unwrap();
        Self { temp }
    }

    fn path(&self, relative: &str) -> PathBuf {
        self.temp.path().join(relative)
    }

    fn text(&self, relative: &str) -> String {
        fs::read_to_string(self.path(relative)).unwrap()
    }

    fn run(&self, staged: &str, wait_pid: u32) -> Instant {
        self.run_with(staged, wait_pid, &[])
    }

    fn run_with(&self, staged: &str, wait_pid: u32, args: &[String]) -> Instant {
        let started = Instant::now();
        let install = self.path("install");
        let exe = install.join("app.cmd");
        let staged = self.path(staged);
        let work = self.path("work");
        let mut child = spawn_apply(&Apply {
            staged: &staged,
            install: &install,
            exe: &exe,
            args,
            wait_pid,
            work: &work,
        })
        .unwrap();
        assert!(child.wait().unwrap().success());
        started
    }

    fn launched(&self) -> bool {
        let deadline = Instant::now() + Duration::from_secs(15);
        while Instant::now() < deadline {
            if self.path("marker.txt").exists() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        false
    }
}

fn finished_process() -> u32 {
    let mut child = Command::new("cmd").args(["/c", "exit", "0"]).spawn().unwrap();
    let pid = child.id();
    child.wait().unwrap();
    pid
}

#[test]
fn the_new_files_replace_the_old_ones_and_the_app_starts_again() {
    let f = Fixture::new();
    f.run("staged", finished_process());
    assert_eq!(f.text("install/a.txt"), "new a");
    assert_eq!(f.text("install/b.txt"), "new b");
    assert_eq!(f.text("install/web/index.html"), "new page");
    assert_eq!(f.text("install/keep.txt"), "keep me", "files the update does not ship are left alone");
    assert!(!f.path("staged").exists(), "staging is cleaned up");
    assert!(!f.path("work/backup").exists(), "the safety copy is removed after success");
    assert!(f.text("work/last-apply.txt").starts_with("ok"));
    assert!(f.launched(), "the application was started again");
}

#[test]
fn files_the_new_version_no_longer_ships_are_removed_from_its_folders() {
    let f = Fixture::new();
    f.run("staged", finished_process());
    assert!(!f.path("install/web/old-5f3a.js").exists(), "old hashed assets do not pile up");
    assert_eq!(f.text("install/web/index.html"), "new page");
    assert_eq!(f.text("install/other/notes.txt"), "not part of the package");
    assert_eq!(f.text("install/keep.txt"), "keep me");
}

#[test]
fn the_app_starts_again_with_the_folders_it_was_started_with() {
    let f = Fixture::new();
    let data = f.path("my data").display().to_string();
    let args = vec!["--data-dir".to_owned(), data.clone(), "--profile-dir".to_owned(), format!("{data}\\")];
    f.run_with("staged", finished_process(), &args);
    assert!(f.launched());
    let marker = loop {
        let text = f.text("marker.txt");
        if !text.trim().is_empty() {
            break text;
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    assert!(marker.contains("--data-dir"), "{marker}");
    assert!(marker.contains(&format!("\"{data}\"")), "{marker}");
    assert!(marker.contains("--profile-dir"), "{marker}");
}

#[test]
fn a_failed_update_puts_the_old_version_back_and_still_starts_the_app() {
    let f = Fixture::new();
    f.run("does-not-exist", finished_process());
    assert_eq!(f.text("install/a.txt"), "old a");
    assert_eq!(f.text("install/web/index.html"), "old page");
    assert!(!f.path("install/b.txt").exists());
    assert!(f.text("work/last-apply.txt").starts_with("failed"));
    assert!(f.launched(), "the old application was started again");
}

#[test]
fn nothing_is_replaced_until_the_running_app_has_exited() {
    let f = Fixture::new();
    let mut running = Command::new("powershell.exe")
        .args(["-NoProfile", "-Command", "Start-Sleep -Seconds 3"])
        .spawn()
        .unwrap();
    let started = f.run("staged", running.id());
    assert!(started.elapsed() >= Duration::from_millis(1500), "the script waited for the process");
    assert!(running.try_wait().unwrap().is_some());
    assert_eq!(f.text("install/a.txt"), "new a");
}

#[test]
fn the_script_uses_only_the_paths_it_is_given() {
    let f = Fixture::new();
    fs::write(f.path("outside.txt"), "untouched").unwrap();
    f.run("staged", finished_process());
    assert_eq!(f.text("outside.txt"), "untouched");
    let _: &Path = &f.path("install");
}
