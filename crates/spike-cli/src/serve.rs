//! Explicit loopback hosting for registered in-place packs and the built app shell.
use std::collections::{HashMap, HashSet};
use std::io::{self, Write};
use std::path::PathBuf;
use std::sync::{mpsc, Arc};

use tonalli_core::pack::{PackError, PackSource, PreadSource};
use tonalli_core::server::{LoopbackServer, PackId, PackRegistry};
use tonalli_core::zim::Zim;

const USAGE: &str =
    "Usage: spike-cli serve [--pack ID=PATH]... [--title-index ID=PATH]... --web BUILT_WEB_ROOT --data-dir PRIVATE_DATA_DIR";

enum Kind {
    Zim,
    Tiles,
}
struct Options {
    packs: Vec<(PackId, PathBuf, Kind)>,
    title_indexes: HashMap<PackId, PathBuf>,
    web: PathBuf,
    data: PathBuf,
}

/// Starts an owned server; dropping it or calling shutdown stops the listener.
pub fn start(arguments: &[String]) -> Result<LoopbackServer, PackError> {
    let options = parse(arguments)?;
    let web = options.web.canonicalize()?;
    if !web.is_dir() || !web.join("index.html").is_file() {
        return Err(PackError::Mismatch(
            "--web must contain a built index.html; run the frontend build first".into(),
        ));
    }
    std::fs::create_dir_all(&options.data)?;
    let data = options.data.canonicalize()?;
    if data.starts_with(&web) {
        return Err(PackError::Mismatch(
            "--data-dir must be outside the served web root".into(),
        ));
    }
    let registry = Arc::new(PackRegistry::default());
    for (id, path, kind) in options.packs {
        // Explicitly selected paths may be SD/USB files; never infer mmap safety.
        let source: Arc<dyn PackSource> = Arc::new(PreadSource::open(&path)?);
        match kind {
            Kind::Zim => {
                let mut zim = Zim::open(source)?;
                if let Some(index) = options.title_indexes.get(&id) {
                    zim = zim.with_title_index(index)?;
                }
                registry.register_zim(id, Arc::new(zim))?;
            }
            Kind::Tiles => registry.register_tiles(id, source)?,
        }
    }
    LoopbackServer::start(registry, data, web)
}

pub fn run(arguments: &[String]) -> Result<(), PackError> {
    let mut server = start(arguments)?;
    let (send, receive) = mpsc::sync_channel(1);
    ctrlc::set_handler(move || {
        let _ = send.try_send(());
    })
    .map_err(|error| PackError::Io(format!("cannot install shutdown signal handler: {error}")))?;
    println!("{}", server.info().launch_url);
    io::stdout().flush()?;
    receive
        .recv()
        .map_err(|error| PackError::Io(error.to_string()))?;
    server.shutdown()
}

fn parse(arguments: &[String]) -> Result<Options, PackError> {
    let mut arguments = arguments.iter();
    let mut packs = Vec::new();
    let mut title_indexes = HashMap::new();
    let mut ids = HashSet::new();
    let mut web = None;
    let mut data = None;
    while let Some(flag) = arguments.next() {
        let value = arguments
            .next()
            .filter(|value| !value.starts_with("--"))
            .ok_or_else(|| invalid(format!("missing value for {flag}")))?;
        match flag.as_str() {
            "--web" if web.is_none() => web = Some(PathBuf::from(value)),
            "--data-dir" if data.is_none() => data = Some(PathBuf::from(value)),
            "--title-index" => {
                let (id, path) = value
                    .split_once('=')
                    .filter(|(_, path)| !path.is_empty())
                    .ok_or_else(|| invalid("--title-index requires ID=PATH"))?;
                let id = PackId::new(id)?;
                if title_indexes.insert(id, PathBuf::from(path)).is_some() {
                    return Err(invalid("duplicate title-index id"));
                }
            }
            "--pack" => {
                let (id, path) = value
                    .split_once('=')
                    .filter(|(_, path)| !path.is_empty())
                    .ok_or_else(|| invalid("--pack requires ID=PATH"))?;
                let id = PackId::new(id)?;
                if !ids.insert(id.as_str().to_owned()) {
                    return Err(invalid("duplicate pack id"));
                }
                let path = PathBuf::from(path);
                let extension = path
                    .extension()
                    .and_then(|extension| extension.to_str())
                    .unwrap_or("");
                let kind = if extension.eq_ignore_ascii_case("zim") {
                    Kind::Zim
                } else if extension.eq_ignore_ascii_case("pmtiles") {
                    Kind::Tiles
                } else {
                    return Err(invalid("supported in-place packs are .zim and .pmtiles"));
                };
                packs.push((id, path, kind));
            }
            _ => return Err(invalid(format!("unknown or repeated option {flag}"))),
        }
    }
    for id in title_indexes.keys() {
        if !packs
            .iter()
            .any(|(pack, _, kind)| pack == id && matches!(kind, Kind::Zim))
        {
            return Err(invalid("--title-index must name a registered .zim pack"));
        }
    }
    Ok(Options {
        packs,
        title_indexes,
        web: web.ok_or_else(|| invalid("--web is required"))?,
        data: data.ok_or_else(|| invalid("--data-dir is required"))?,
    })
}

fn invalid(message: impl std::fmt::Display) -> PackError {
    PackError::Mismatch(format!("{message}\n{USAGE}"))
}
