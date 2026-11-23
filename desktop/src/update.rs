//! Windows self-update: verify and unpack the downloaded package, then hand the file swap to a small
//! script that runs after this process has exited.

use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

use tonalli_core::update::{app_version, current_platform, unpack, Store};

const SCRIPT: &str = include_str!("update_apply.ps1");
const SCRIPT_NAME: &str = "apply-update.ps1";
const PACKAGE_FOLDER: &str = "Windows-x64/";
const APPLICATION: &str = "tonalli-desktop.exe";
const MAX_UNPACKED: u64 = 2 << 30;
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;

pub struct Apply<'a> {
    /// Folder holding the unpacked new files.
    pub staged: &'a Path,
    /// The installed application folder to update.
    pub install: &'a Path,
    /// The application to start again afterwards.
    pub exe: &'a Path,
    /// The arguments this process was started with (without the program name), so the new
    /// version reopens the same data and profile folders.
    pub args: &'a [String],
    /// This process; the script waits for it to exit before touching any file.
    pub wait_pid: u32,
    /// A private folder for the script, the safety copy and the result log.
    pub work: &'a Path,
}

pub fn spawn_apply(apply: &Apply) -> std::io::Result<Child> {
    std::fs::create_dir_all(apply.work)?;
    let script = apply.work.join(SCRIPT_NAME);
    std::fs::write(&script, SCRIPT)?;
    let mut command = Command::new("powershell.exe");
    command
        .args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-File"])
        .arg(&script)
        .arg("-WaitPid")
        .arg(apply.wait_pid.to_string())
        .arg("-Source")
        .arg(apply.staged)
        .arg("-Target")
        .arg(apply.install)
        .arg("-Exe")
        .arg(apply.exe)
        .arg("-Work")
        .arg(apply.work);
    if !apply.args.is_empty() {
        command.arg("-AppArgs").arg(command_line(apply.args));
    }
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW | CREATE_NEW_PROCESS_GROUP)
        .spawn()
}

/// One Windows command line from separate arguments, quoted so a path with spaces, quotes or a
/// trailing backslash reaches the new process as the same single argument.
fn command_line(args: &[String]) -> String {
    args.iter()
        .map(|arg| {
            let mut quoted = String::from("\"");
            let mut backslashes = 0;
            for c in arg.chars() {
                match c {
                    '\\' => backslashes += 1,
                    '"' => {
                        quoted.push_str(&"\\".repeat(backslashes * 2 + 1));
                        quoted.push('"');
                        backslashes = 0;
                    }
                    _ => {
                        quoted.push_str(&"\\".repeat(backslashes));
                        quoted.push(c);
                        backslashes = 0;
                    }
                }
            }
            quoted.push_str(&"\\".repeat(backslashes * 2));
            quoted.push('"');
            quoted
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Re-verifies the downloaded package against the signed manifest and unpacks it into a fresh folder.
pub fn stage(data: &Path) -> Result<PathBuf, String> {
    let platform = current_platform().ok_or("This device has no update package.")?;
    let installed = app_version().map_err(|error| error.to_string())?;
    let store = Store::new(data);
    let ready = store
        .ready(&installed, platform)
        .map_err(|error| error.to_string())?
        .ok_or("There is no verified update to install.")?;
    let staged = data.join("updates").join("staged");
    if staged.exists() {
        std::fs::remove_dir_all(&staged).map_err(|error| error.to_string())?;
    }
    std::fs::create_dir_all(data.join("updates")).map_err(|error| error.to_string())?;
    unpack(&ready.path, &staged, PACKAGE_FOLDER, APPLICATION, MAX_UNPACKED)
        .map_err(|error| error.to_string())?;
    Ok(staged)
}

#[cfg(test)]
mod tests {
    use super::command_line;

    fn line(args: &[&str]) -> String {
        command_line(&args.iter().map(|a| (*a).to_owned()).collect::<Vec<_>>())
    }

    #[test]
    fn each_argument_stays_one_argument() {
        assert_eq!(line(&["--data-dir", "C:\\my data"]), "\"--data-dir\" \"C:\\my data\"");
        assert_eq!(line(&["C:\\my data\\"]), "\"C:\\my data\\\\\"");
        assert_eq!(line(&["say \"hi\""]), "\"say \\\"hi\\\"\"");
        assert_eq!(line(&["a\\\"b"]), "\"a\\\\\\\"b\"");
        assert_eq!(line(&[""]), "\"\"");
    }
}
