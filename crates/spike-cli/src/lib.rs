//! Disposable feasibility harness; reusable commands stay callable from tests.

pub mod add;
#[cfg(feature = "ai")]
pub mod ai;
pub mod catalog;
pub mod fetch;
pub mod index;
pub mod manifest;
pub mod rerank;
pub mod serve;
pub mod update;

use std::path::PathBuf;
use tonalli_core::pack::PackError;

const USAGE: &str = "Usage: spike-cli fetch (--id PACK_ID | --all) [--manifest PATH]\n       spike-cli add PATH_OR_FOLDER [--manifest PATH]\n       spike-cli index --zim PATH --out NEWDIR [--threads 1..4]\n       spike-cli ai-rerank --id PACK_ID --index-authority FROZEN_RECEIPT --out NEWDIR --export-only\n       spike-cli serve [--pack ID=PATH]... --web BUILT_WEB_ROOT --data-dir PRIVATE_DATA_DIR\n       spike-cli ai-bench --id PACK_ID --out NEWDIR [--manifest PATH] [--smoke-seconds 1..60] (optional ai feature)\n       spike-cli catalog-keygen --out NEW_PRIVATE_KEY\n       spike-cli catalog-sign --catalog JSON --key PRIVATE_KEY --out NEW_SIGNATURE\n       spike-cli update-keygen --out NEW_PRIVATE_KEY\n       spike-cli update-manifest --version V --published DATE --page URL --base-url URL --key PRIVATE_KEY --out-dir DIR --asset PLATFORM=FILE...\n--all skips deferred packs; use --id to explicitly fetch one.";

pub fn run(args: impl IntoIterator<Item = String>) -> Result<(), PackError> {
    let mut args: Vec<_> = args.into_iter().collect();
    if args.is_empty() || args == ["--help"] || args == ["-h"] {
        println!("{USAGE}");
        return Ok(());
    }
    if args
        .first()
        .is_some_and(|command| matches!(command.as_str(), "catalog-keygen" | "catalog-sign"))
    {
        return catalog::run(&args);
    }
    if args
        .first()
        .is_some_and(|command| matches!(command.as_str(), "update-keygen" | "update-manifest"))
    {
        return update::run(&args);
    }
    if args.first().is_some_and(|command| command == "serve") {
        return serve::run(&args[1..]);
    }
    if args.first().is_some_and(|command| command == "index") {
        return index::run(&args[1..]);
    }
    if args.first().is_some_and(|command| command == "ai-rerank") {
        return rerank::run(&args[1..]);
    }
    if args.first().is_some_and(|command| command == "ai-bench") {
        #[cfg(feature = "ai")]
        return ai::run(&args[1..]);
        #[cfg(not(feature = "ai"))]
        return Err(PackError::Mismatch(
            "ai-bench requires the optional ai feature".into(),
        ));
    }
    let mut manifest_path = PathBuf::from("docs/spike/packs.toml");
    if let Some(index) = args.iter().position(|arg| arg == "--manifest") {
        if index + 1 >= args.len() {
            return Err(PackError::Mismatch(USAGE.into()));
        }
        manifest_path = PathBuf::from(args.remove(index + 1));
        args.remove(index);
    }
    match args.as_slice() {
        [command, source] if command == "add" && !source.starts_with("--") => {
            for id in add::add_path(&manifest_path, &PathBuf::from(source))? {
                println!("Registered {id}");
            }
        }
        [command, flag] if command == "fetch" && flag == "--all" => {
            let manifest = manifest::Manifest::load(&manifest_path)?;
            for pack in &manifest.packs {
                if pack.deferred {
                    println!("Skipped deferred pack {}", pack.id);
                    continue;
                }
                println!("Verifying/fetching {} ({} bytes)", pack.id, pack.bytes);
                let path = fetch::fetch_pack(pack, manifest::manifest_dir(&manifest_path))?;
                println!("Verified {}: {}", pack.id, path.display());
            }
        }
        [command, flag, id] if command == "fetch" && flag == "--id" => {
            let manifest = manifest::Manifest::load(&manifest_path)?;
            let pack = manifest
                .packs
                .iter()
                .find(|pack| pack.id == *id)
                .ok_or_else(|| PackError::NotFound(id.clone()))?;
            println!("Verifying/fetching {} ({} bytes)", pack.id, pack.bytes);
            let path = fetch::fetch_pack(pack, manifest::manifest_dir(&manifest_path))?;
            println!("Verified {}: {}", pack.id, path.display());
        }
        _ => return Err(PackError::Mismatch(USAGE.into())),
    }
    Ok(())
}
