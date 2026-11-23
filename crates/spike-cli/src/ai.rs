//! Explicit desktop generation measurement; never part of ordinary model-free tests.

use std::path::PathBuf;
use tonalli_core::pack::PackError;

mod measurement;
pub(crate) mod model;
mod runner;

const USAGE: &str =
    "Usage: spike-cli ai-bench --id PACK_ID --out NEWDIR [--manifest PATH] [--smoke-seconds 1..60]";

#[derive(Debug)]
struct Config {
    id: String,
    manifest: PathBuf,
    output: PathBuf,
    seconds: u64,
    smoke: bool,
}

fn parse(args: &[String]) -> Result<Config, PackError> {
    let (mut id, mut output, mut manifest, mut seconds) = (None, None, None, None);
    let mut args = args.iter();
    while let Some(flag) = args.next() {
        let value = args
            .next()
            .filter(|value| !value.is_empty() && !value.starts_with("--"))
            .ok_or_else(|| PackError::Mismatch(USAGE.into()))?;
        match flag.as_str() {
            "--id" if id.is_none() && value.len() <= 256 => id = Some(value.clone()),
            "--out" if output.is_none() => output = Some(PathBuf::from(value)),
            "--manifest" if manifest.is_none() => manifest = Some(PathBuf::from(value)),
            "--smoke-seconds" if seconds.is_none() => {
                let value: u64 = value
                    .parse()
                    .map_err(|_| PackError::Mismatch(USAGE.into()))?;
                if !(1..=60).contains(&value) {
                    return Err(PackError::Mismatch(USAGE.into()));
                }
                seconds = Some(value);
            }
            _ => return Err(PackError::Mismatch(USAGE.into())),
        }
    }
    Ok(Config {
        id: id.ok_or_else(|| PackError::Mismatch(USAGE.into()))?,
        output: output.ok_or_else(|| PackError::Mismatch(USAGE.into()))?,
        manifest: manifest.unwrap_or_else(|| PathBuf::from("docs/spike/packs.toml")),
        seconds: seconds.unwrap_or(600),
        smoke: seconds.is_some(),
    })
}

pub fn run(args: &[String]) -> Result<(), PackError> {
    runner::run(parse(args)?)
}

#[cfg(test)]
mod tests;
