//! Explicit Windows/desktop full-text builder; never replace an existing output.
use std::{fs, path::PathBuf, sync::Arc, time::Instant};

use serde::{Deserialize, Serialize};
use tonalli_core::{
    index::{self, Index, IndexMeta},
    pack::{PackError, PreadSource},
    zim::Zim,
};

const USAGE: &str = "Usage: spike-cli index --zim PATH --out NEWDIR [--threads 1..4]";

#[derive(Serialize)]
struct QueryResult {
    query: &'static str,
    hits: Vec<SearchHit>,
}

#[derive(Serialize)]
struct SearchHit {
    title: String,
    url: String,
}

#[derive(Serialize)]
struct Report {
    version: u32,
    zim: PathBuf,
    output: PathBuf,
    requested_threads: usize,
    extraction_threads: usize,
    build_elapsed_ms: u128,
    documents: u64,
    generation: String,
    generation_bytes: u64,
    index_meta: IndexMeta,
    queries: Vec<QueryResult>,
}

#[derive(Deserialize)]
struct TantivyMeta {
    segments: Vec<Segment>,
}

#[derive(Deserialize)]
struct Segment {
    max_doc: u64,
    deletes: Option<serde_json::Value>,
}

pub fn run(args: &[String]) -> Result<(), PackError> {
    let mut zim_path = None;
    let mut output = None;
    let mut threads = None;
    let mut pairs = args.chunks_exact(2);
    for pair in &mut pairs {
        if pair[1].starts_with("--") {
            return Err(PackError::Mismatch(USAGE.into()));
        }
        match pair[0].as_str() {
            "--zim" if zim_path.is_none() => zim_path = Some(PathBuf::from(&pair[1])),
            "--out" if output.is_none() => output = Some(PathBuf::from(&pair[1])),
            "--threads" if threads.is_none() => {
                threads = Some(
                    pair[1]
                        .parse::<usize>()
                        .map_err(|_| PackError::Mismatch(USAGE.into()))?,
                );
            }
            _ => return Err(PackError::Mismatch(USAGE.into())),
        }
    }
    let threads = threads.unwrap_or(4);
    let (Some(zim_path), Some(output)) = (zim_path, output) else {
        return Err(PackError::Mismatch(USAGE.into()));
    };
    if !pairs.remainder().is_empty() || !(1..=4).contains(&threads) {
        return Err(PackError::Mismatch(USAGE.into()));
    }
    // create_dir is exclusive, including for an empty directory or dangling link.
    // Opening the source first avoids reserving output for a missing/invalid ZIM.
    let zim = Zim::open(Arc::new(PreadSource::open(&zim_path)?))?;
    fs::create_dir(&output)?;
    let started = Instant::now();
    let index_meta = index::build(&zim, &output, threads)?;
    let build_elapsed_ms = started.elapsed().as_millis();
    let index = Index::open(&output, &zim)?;
    let generation = fs::read_to_string(output.join("CURRENT"))?;
    let directory = output.join(&generation);
    let meta: TantivyMeta = serde_json::from_slice(&fs::read(directory.join("meta.json"))?)
        .map_err(|error| PackError::Corrupt(error.to_string()))?;
    let mut documents = 0_u64;
    for segment in meta.segments {
        if segment.deletes.is_some() {
            return Err(PackError::Corrupt(
                "new index unexpectedly contains deletions".into(),
            ));
        }
        documents = documents
            .checked_add(segment.max_doc)
            .ok_or_else(|| PackError::Corrupt("index document count overflow".into()))?;
    }
    let mut generation_bytes = 0_u64;
    for entry in fs::read_dir(&directory)? {
        let entry = entry?;
        if !entry.file_type()?.is_file() {
            return Err(PackError::Corrupt(
                "index generation contains a non-file".into(),
            ));
        }
        generation_bytes = generation_bytes
            .checked_add(entry.metadata()?.len())
            .ok_or_else(|| PackError::Corrupt("index byte count overflow".into()))?;
    }
    let queries = ["solar system", "photosynthesis", "Mexican history"]
        .into_iter()
        .map(|query| {
            Ok(QueryResult {
                query,
                hits: index
                    .search(query, 5)?
                    .into_iter()
                    .map(|hit| SearchHit {
                        title: hit.title,
                        url: hit.url,
                    })
                    .collect(),
            })
        })
        .collect::<Result<Vec<_>, PackError>>()?;
    let report = Report {
        version: 1,
        zim: zim_path,
        output,
        requested_threads: threads,
        extraction_threads: threads
            .min(std::thread::available_parallelism().map_or(1, usize::from)),
        build_elapsed_ms,
        documents,
        generation,
        generation_bytes,
        index_meta,
        queries,
    };
    println!(
        "{}",
        serde_json::to_string(&report).map_err(|error| PackError::Corrupt(error.to_string()))?
    );
    Ok(())
}
