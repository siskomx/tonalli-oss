use std::collections::{HashMap, HashSet};
use std::net::IpAddr;
use std::path::PathBuf;

pub const USAGE: &str = "tonalli-desktop [--web BUILT_WEB_ROOT] [--data-dir PRIVATE_DATA_DIR] [--profile-dir PRIVATE_WEBVIEW_PROFILE] [--pack ID=PATH]... [--title-index ID=PATH]... [--iso-inner /EXPLICIT/PATH.zim] [--ai-model qwen3-1.7b-q4-k-m] [--headless PRIVATE_LAN_IPV4|auto --status-file PRIVATE_PATH --share-pack ID...]";

/// Window-less LAN sharing of explicitly named installed packs. Nothing is shared unless all three are given.
pub struct Headless {
    /// `None` means the one eligible private LAN address; several choices are refused.
    pub address: Option<IpAddr>,
    pub status_file: PathBuf,
    pub share_packs: Vec<String>,
}

pub struct Options {
    pub headless: Option<Headless>,
    pub web: PathBuf,
    pub data: PathBuf,
    pub profile: PathBuf,
    pub packs: Vec<(String, PathBuf)>,
    pub title_indexes: HashMap<String, PathBuf>,
    pub iso_inner: Option<String>,
    pub ai_model: Option<String>,
}

impl Options {
    pub fn parse(arguments: impl IntoIterator<Item = String>) -> Result<Self, String> {
        let mut arguments = arguments.into_iter();
        let (mut web, mut data, mut profile, mut iso_inner) = (None, None, None, None);
        let mut packs = Vec::new();
        let mut ids = HashSet::new();
        let mut title_indexes = HashMap::new();
        let mut ai_model = None;
        let (mut headless, mut status_file, mut share_packs) = (None, None, Vec::<String>::new());
        while let Some(flag) = arguments.next() {
            let value = arguments
                .next()
                .filter(|v| !v.is_empty() && !v.starts_with("--"))
                .ok_or_else(|| format!("Missing value for {flag}. {USAGE}"))?;
            match flag.as_str() {
                "--ai-model" if ai_model.is_none() && value == "qwen3-1.7b-q4-k-m" => {
                    ai_model = Some(value)
                }
                "--headless" if headless.is_none() => {
                    let address = if value == "auto" {
                        None
                    } else {
                        let ip = value
                            .parse::<IpAddr>()
                            .ok()
                            .filter(|ip| ip.is_ipv4() && !ip.is_loopback() && !ip.is_unspecified())
                            .ok_or("--headless needs a private LAN IPv4 address or auto")?;
                        Some(ip)
                    };
                    headless = Some(address);
                }
                "--status-file" if status_file.is_none() => status_file = Some(PathBuf::from(value)),
                "--share-pack" => {
                    if share_packs.contains(&value) {
                        return Err("Duplicate --share-pack ID".into());
                    }
                    share_packs.push(value);
                }
                "--web" if web.is_none() => web = Some(PathBuf::from(value)),
                "--data-dir" if data.is_none() => data = Some(PathBuf::from(value)),
                "--profile-dir" if profile.is_none() => profile = Some(PathBuf::from(value)),
                "--iso-inner" if iso_inner.is_none() => {
                    let relative = value.strip_prefix('/').unwrap_or(&value);
                    if value.len() > 4096
                        || value.contains('\\')
                        || value.chars().any(char::is_control)
                        || relative
                            .split('/')
                            .any(|v| v.is_empty() || matches!(v, "." | ".."))
                    {
                        return Err(
                            "ISO inner path must be explicit, bounded and case-sensitive".into(),
                        );
                    }
                    iso_inner = Some(value);
                }
                "--pack" | "--title-index" => {
                    let (id, path) = value
                        .split_once('=')
                        .filter(|(id, path)| !id.is_empty() && !path.is_empty())
                        .ok_or_else(|| format!("{flag} requires ID=PATH"))?;
                    if flag == "--pack" {
                        if id.starts_with("reader-") {
                            return Err("reader-* IDs belong to the native content catalog".into());
                        }
                        if !ids.insert(id.to_owned()) {
                            return Err("Duplicate pack ID".into());
                        }
                        packs.push((id.to_owned(), PathBuf::from(path)));
                    } else if title_indexes
                        .insert(id.to_owned(), PathBuf::from(path))
                        .is_some()
                    {
                        return Err("Duplicate title-index ID".into());
                    }
                }
                _ => return Err(format!("Unknown or repeated option {flag}. {USAGE}")),
            }
        }
        if title_indexes.keys().any(|id| !ids.contains(id)) {
            return Err("Title index must name a registered pack".into());
        }
        let headless = match (headless, status_file, share_packs.is_empty()) {
            (None, None, true) => None,
            (Some(address), Some(status_file), false) => Some(Headless {
                address,
                status_file,
                share_packs,
            }),
            _ => {
                return Err(
                    "Headless sharing needs --headless, --status-file and at least one --share-pack"
                        .into(),
                )
            }
        };
        let data = data.map_or_else(default_data_dir, Ok)?;
        Ok(Self {
            headless,
            web: web.map_or_else(
                || {
                    std::env::current_exe()
                        .ok()
                        .and_then(|path| path.parent().map(|parent| parent.join("web")))
                        .ok_or_else(|| "Bundled web directory is unavailable".to_owned())
                },
                Ok,
            )?,
            profile: profile.unwrap_or_else(|| data.join("webview2")),
            data,
            packs,
            title_indexes,
            iso_inner,
            ai_model,
        })
    }
}

fn default_data_dir() -> Result<PathBuf, String> {
    #[cfg(windows)]
    let base = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("USERPROFILE").map(|home| PathBuf::from(home).join("AppData/Local"))
        });
    #[cfg(not(windows))]
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share")));
    base.filter(|path| path.is_absolute())
        .map(|path| path.join("Tonalli"))
        .ok_or_else(|| "Private app data directory is unavailable; use --data-dir".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(values: &[&str]) -> Result<Options, String> {
        Options::parse(values.iter().map(|v| (*v).to_owned()))
    }

    #[test]
    fn reader_ids_are_reserved_for_native_catalog_ownership() {
        assert!(parse(&["--pack", "reader-1=selected.zim"]).is_err());
    }

    #[test]
    fn ordinary_launch_needs_no_command_line_paths() {
        let options = parse(&[]).expect("normal app launch");
        assert!(options.web.is_absolute());
        assert_eq!(options.web.file_name().unwrap(), "web");
        assert!(options.data.is_absolute());
        assert_eq!(options.data.file_name().unwrap(), "Tonalli");
        assert_eq!(options.profile, options.data.join("webview2"));
        assert!(options.ai_model.is_none());
        assert!(options.packs.is_empty());
    }

    #[test]
    fn diagnostic_paths_override_only_their_own_defaults() {
        let web = parse(&["--web", "diagnostic-web"]).expect("web override");
        assert_eq!(web.web, PathBuf::from("diagnostic-web"));
        assert!(web.data.is_absolute());
        let data = parse(&["--data-dir", "diagnostic-data"]).expect("data override");
        assert_eq!(data.data, PathBuf::from("diagnostic-data"));
        assert!(data.web.is_absolute());
    }

    #[test]
    fn explicit_local_inputs_and_private_profile_are_preserved() {
        let options = parse(&[
            "--web",
            "built UI",
            "--data-dir",
            "private",
            "--profile-dir",
            "run/profile",
            "--pack",
            "probe=C:/My packs/knots.zim",
            "--title-index",
            "probe=C:/index/table.cti",
            "--iso-inner",
            "/Packs/Fixture.zim",
        ])
        .expect("valid inputs");
        assert_eq!(options.packs[0].1, PathBuf::from("C:/My packs/knots.zim"));
        assert_eq!(options.iso_inner.as_deref(), Some("/Packs/Fixture.zim"));
        assert_eq!(options.profile, PathBuf::from("run/profile"));
        assert_eq!(
            parse(&["--web", "web", "--data-dir", "data"])
                .expect("defaults")
                .profile,
            PathBuf::from("data/webview2")
        );
    }

    #[test]
    fn unsafe_or_ambiguous_arguments_are_rejected() {
        for tail in [
            vec!["--iso-inner", "../private"],
            vec!["--iso-inner", "/a//b"],
            vec!["--iso-inner", "/a\\b"],
            vec!["--iso-inner", "/"],
            vec!["--web", "another"],
            vec!["--pack", "same=one.zim", "--pack", "same=two.zim"],
            vec!["--title-index", "missing=table.cti"],
            vec!["--serve", "0.0.0.0"],
            vec!["--pack"],
        ] {
            let mut args = vec!["--web", "web", "--data-dir", "private"];
            args.extend(tail);
            assert!(parse(&args).is_err());
        }
    }

    #[test]
    fn headless_sharing_needs_an_explicit_private_address_status_file_and_content() {
        let base = ["--web", "web", "--data-dir", "private"];
        let with = |tail: &[&str]| {
            let mut args = base.to_vec();
            args.extend_from_slice(tail);
            parse(&args)
        };
        assert!(parse(&base).unwrap().headless.is_none());
        let options = with(&[
            "--headless",
            "192.168.1.20",
            "--status-file",
            "C:/private/status.json",
            "--share-pack",
            "book",
            "--share-pack",
            "map",
        ])
        .unwrap();
        let headless = options.headless.unwrap();
        assert_eq!(headless.address, Some("192.168.1.20".parse().unwrap()));
        assert_eq!(headless.status_file, PathBuf::from("C:/private/status.json"));
        assert_eq!(headless.share_packs, ["book", "map"]);
        let auto = with(&["--headless", "auto", "--status-file", "s.json", "--share-pack", "book"])
            .unwrap()
            .headless
            .unwrap();
        assert_eq!(auto.address, None);
        for tail in [
            vec!["--headless", "0.0.0.0", "--status-file", "s.json", "--share-pack", "book"],
            vec!["--headless", "127.0.0.1", "--status-file", "s.json", "--share-pack", "book"],
            vec!["--headless", "::1", "--status-file", "s.json", "--share-pack", "book"],
            vec!["--headless", "example.test", "--status-file", "s.json", "--share-pack", "book"],
            vec!["--headless", "auto", "--share-pack", "book"],
            vec!["--headless", "auto", "--status-file", "s.json"],
            vec!["--headless", "auto", "--status-file", "s.json", "--share-pack", "book", "--share-pack", "book"],
            vec!["--headless", "auto", "--headless", "auto", "--status-file", "s.json", "--share-pack", "book"],
            vec!["--status-file", "s.json"],
            vec!["--share-pack", "book"],
        ] {
            assert!(with(&tail).is_err(), "{tail:?}");
        }
    }

    #[test]
    fn ai_is_opt_in_and_only_accepts_the_trusted_installed_catalog_id() {
        let plain = parse(&["--web", "web", "--data-dir", "private"]).unwrap();
        assert!(plain.ai_model.is_none());
        let selected = parse(&[
            "--web",
            "web",
            "--data-dir",
            "private",
            "--ai-model",
            "qwen3-1.7b-q4-k-m",
        ])
        .unwrap();
        assert_eq!(selected.ai_model.as_deref(), Some("qwen3-1.7b-q4-k-m"));
        for value in [
            "../model.gguf",
            "C:/user/model.gguf",
            "unknown",
            "qwen3-4b-q4-k-m",
        ] {
            assert!(
                parse(&["--web", "web", "--data-dir", "private", "--ai-model", value]).is_err()
            );
        }
        assert!(parse(&[
            "--web",
            "web",
            "--data-dir",
            "private",
            "--ai-model",
            "qwen3-1.7b-q4-k-m",
            "--ai-model",
            "qwen3-1.7b-q4-k-m"
        ])
        .is_err());
    }
}
