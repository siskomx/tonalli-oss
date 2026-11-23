use std::ops::ControlFlow;
use std::{fs, fs::OpenOptions, path::Path};

use fs2::FileExt;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use tantivy::{doc, TantivyDocument};

use super::{
    directory, guarded, html, identity, index_error, schema, storage, writer::BuildWriter, Index,
    IndexMeta, WRITER_BUDGET,
};
use crate::{pack::PackError, zim::Zim};

#[cfg(test)]
#[path = "build_tests.rs"]
mod tests;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuildControl {
    Continue,
    Pause,
    Cancel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BuildPhase {
    Checking,
    Indexing,
    Validating,
    Activating,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BuildProgress {
    pub phase: BuildPhase,
    pub processed_articles: u64,
    pub committed_articles: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BuildOutcome {
    Ready(IndexMeta),
    Paused(BuildProgress),
    Cancelled,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Checkpoint {
    version: u32,
    meta: IndexMeta,
    articles: u64,
    complete: bool,
}

impl Checkpoint {
    fn payload(&self) -> Result<String, PackError> {
        serde_json::to_string(self).map_err(index_error)
    }
    fn progress(&self, phase: BuildPhase) -> BuildProgress {
        BuildProgress {
            phase,
            processed_articles: self.articles,
            committed_articles: self.articles,
        }
    }
}

/// App-internal output only. One caller owns the build until all native workers exit.
/// The callback runs on the coordinator without writer locks. Checksum and merge
/// operations are synchronous boundaries; Pause is confirmed only after commit/exit.
pub fn build_controlled(
    zim: &Zim,
    out: &Path,
    threads: usize,
    progress: impl FnMut(BuildProgress) -> Result<BuildControl, PackError>,
) -> Result<BuildOutcome, PackError> {
    controlled(zim, out, threads, WRITER_BUDGET, progress).map(|(outcome, _)| outcome)
}

pub(super) fn controlled(
    zim: &Zim,
    out: &Path,
    threads: usize,
    queue_budget: usize,
    mut progress: impl FnMut(BuildProgress) -> Result<BuildControl, PackError>,
) -> Result<(BuildOutcome, usize), PackError> {
    guarded(|| {
        if threads == 0 {
            return Err(PackError::Mismatch(
                "indexing needs at least one worker".into(),
            ));
        }
        let threads = threads
            .min(std::thread::available_parallelism().map_or(1, usize::from))
            .min(8);
        fs::create_dir_all(out)?;
        let _lock = build_lock(out)?;
        let meta = identity(zim)?;
        let existing = match fs::symlink_metadata(out.join("BUILDING")) {
            Ok(_) => Some(storage::pointed_path(out, "BUILDING")?),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(e.into()),
        };
        let mut checkpoint = Checkpoint {
            version: 1,
            meta: meta.clone(),
            articles: 0,
            complete: false,
        };
        let mut opened = None;
        let mut early = None;
        let mut activated = false;
        if let Some(path) = &existing {
            activated = match fs::symlink_metadata(out.join("CURRENT")) {
                Ok(_) => storage::generation(out)? == *path,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
                Err(e) => return Err(e.into()),
            };
            let missing = match fs::symlink_metadata(path) {
                Ok(_) => {
                    storage::check_generation(path)?;
                    false
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => true,
                Err(e) => return Err(e.into()),
            };
            let empty = !missing && fs::read_dir(path)?.next().is_none();
            if missing || empty {
                if progress(checkpoint.progress(BuildPhase::Checking))? == BuildControl::Cancel
                    && !activated
                {
                    discard(out, path)?;
                    return Ok((BuildOutcome::Cancelled, 0));
                }
                return Err(PackError::Corrupt(
                    "interrupted index cleanup needs explicit Cancel".into(),
                ));
            }
            let owner = fs::symlink_metadata(path.join(storage::OWNER));
            match owner {
                Ok(_) if storage::read_owner(path)? != meta => {
                    return Err(PackError::Mismatch(
                        "index stage owner source/profile differs".into(),
                    ));
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound && activated => {}
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e.into()),
                Err(_) => {
                    return Err(PackError::Corrupt(
                        "BUILDING does not own this generation".into(),
                    ))
                }
                Ok(_) => {}
            }
            let loaded = (|| {
                if storage::read_meta(path)? != meta {
                    return Err(PackError::Mismatch(
                        "index checkpoint source/profile differs".into(),
                    ));
                }
                let index = tantivy::Index::open_in_dir(path).map_err(index_error)?;
                let saved = read_checkpoint(&index, &meta)?;
                Ok((index, saved))
            })();
            let state = loaded
                .as_ref()
                .map(|(_, saved)| saved.progress(BuildPhase::Checking))
                .unwrap_or_else(|_| checkpoint.progress(BuildPhase::Checking));
            let requested = progress(state)?;
            if requested == BuildControl::Cancel && !activated {
                drop(loaded);
                discard(out, path)?;
                return Ok((BuildOutcome::Cancelled, 0));
            }
            let (index, saved) = loaded?;
            checkpoint = saved;
            opened = Some(index);
            early = Some(requested);
        }
        let early = match early {
            Some(control) => control,
            None => progress(checkpoint.progress(BuildPhase::Checking))?,
        };
        if early == BuildControl::Cancel && !activated {
            drop(opened);
            if let Some(path) = existing {
                discard(out, &path)?;
            }
            return Ok((BuildOutcome::Cancelled, 0));
        }
        let checked = zim.verify_md5_controlled(|_, _| {
            match progress(checkpoint.progress(BuildPhase::Checking)) {
                Ok(BuildControl::Continue) => ControlFlow::Continue(()),
                requested => ControlFlow::Break(requested),
            }
        })?;
        if let ControlFlow::Break(requested) = checked {
            let control = requested?;
            drop(opened);
            if activated {
                return Err(PackError::PublicationUncertain(
                    "index already active; checksum checking stopped".into(),
                ));
            }
            return match control {
                BuildControl::Pause => Ok((
                    BuildOutcome::Paused(checkpoint.progress(BuildPhase::Checking)),
                    0,
                )),
                BuildControl::Cancel => {
                    if let Some(path) = existing {
                        discard(out, &path)?;
                    }
                    Ok((BuildOutcome::Cancelled, 0))
                }
                BuildControl::Continue => {
                    Err(PackError::Corrupt("invalid checksum stop control".into()))
                }
            };
        }
        if identity(zim)? != meta {
            return Err(PackError::Mismatch(
                "index source changed during verification".into(),
            ));
        }
        let (schema, title, body, url) = schema();
        let path = if let Some(path) = existing {
            path
        } else {
            let staged = tempfile::Builder::new()
                .prefix("generation-")
                .tempdir_in(out)?;
            let directory =
                tantivy::directory::MmapDirectory::open(staged.path()).map_err(index_error)?;
            let index = tantivy::Index::create(
                directory::DiagnosticDirectory::new(directory),
                schema.clone(),
                tantivy::IndexSettings::default(),
            )
            .map_err(index_error)?;
            storage::write_meta(staged.path(), &meta)?;
            storage::write_owner(staged.path(), &meta)?;
            let writer = index
                .writer_with_num_threads::<TantivyDocument>(1, WRITER_BUDGET)
                .map_err(index_error)?;
            BuildWriter::new(writer, queue_budget).finish_checkpoint(checkpoint.payload()?)?;
            let path = staged.keep();
            storage::point(out, "BUILDING", &path)?;
            opened = Some(index);
            path
        };
        let index = opened.ok_or_else(|| PackError::Corrupt("missing staged index".into()))?;
        if index.schema() != schema {
            return Err(PackError::Mismatch(
                "index checkpoint schema differs".into(),
            ));
        }
        if !index.validate_checksum().map_err(index_error)?.is_empty() {
            return Err(PackError::Corrupt(
                "index checkpoint checksum failed".into(),
            ));
        }
        let mut entries = zim.canonical_article_entries();
        for _ in 0..checkpoint.articles {
            entries
                .next()
                .ok_or_else(|| PackError::Corrupt("index checkpoint exceeds source".into()))??;
        }
        if checkpoint.complete && entries.next().transpose()?.is_some() {
            return Err(PackError::Corrupt(
                "completed index checkpoint omits source articles".into(),
            ));
        }
        if activated {
            if !checkpoint.complete {
                return Err(PackError::Corrupt(
                    "CURRENT points to an unfinished build".into(),
                ));
            }
            finish_publication(out, &path)?;
            return Ok((BuildOutcome::Ready(meta), 0));
        }
        if early == BuildControl::Pause {
            return Ok((
                BuildOutcome::Paused(checkpoint.progress(BuildPhase::Checking)),
                0,
            ));
        }
        let mut drains = 0;
        if !checkpoint.complete {
            let writer = index
                .writer_with_num_threads::<TantivyDocument>(1, WRITER_BUDGET)
                .map_err(index_error)?;
            let mut writer = BuildWriter::new(writer, queue_budget);
            let pool = rayon::ThreadPoolBuilder::new()
                .num_threads(threads)
                .build()
                .map_err(index_error)?;
            let mut committed = checkpoint.articles;
            loop {
                let state = BuildProgress {
                    phase: BuildPhase::Indexing,
                    processed_articles: checkpoint.articles,
                    committed_articles: committed,
                };
                match progress(state)? {
                    BuildControl::Pause => {
                        drains += writer.finish_checkpoint(checkpoint.payload()?)?;
                        return Ok((
                            BuildOutcome::Paused(checkpoint.progress(BuildPhase::Indexing)),
                            drains,
                        ));
                    }
                    BuildControl::Cancel => {
                        writer.abort()?;
                        drop(index);
                        discard(out, &path)?;
                        return Ok((BuildOutcome::Cancelled, drains));
                    }
                    BuildControl::Continue => {}
                }
                // At most one extraction per worker is retained. Ordered submission
                // makes the commit payload an exact contiguous source cursor.
                let batch = entries
                    .by_ref()
                    .take(threads)
                    .collect::<Result<Vec<_>, _>>()?;
                if batch.is_empty() {
                    break;
                }
                let docs = pool.install(|| {
                    batch
                        .par_iter()
                        .map(|entry| {
                            let article = zim.article(&entry.url)?;
                            let text = html::article_text(&article.mime_type, &article.bytes)?;
                            let bytes = text
                                .len()
                                .saturating_add(article.title.len())
                                .saturating_add(article.url.len());
                            Ok::<_, PackError>((
                                doc!(title => article.title, body => text, url => article.url),
                                bytes,
                            ))
                        })
                        .collect::<Result<Vec<_>, _>>()
                })?;
                for (document, bytes) in docs {
                    checkpoint.articles = checkpoint
                        .articles
                        .checked_add(1)
                        .ok_or_else(|| PackError::Corrupt("index article count overflow".into()))?;
                    if writer.submit_checkpoint(document, bytes, Some(checkpoint.payload()?))? {
                        committed = checkpoint.articles;
                    }
                }
            }
            checkpoint.complete = true;
            drains += writer.finish_checkpoint(checkpoint.payload()?)?;
        }
        for phase in [BuildPhase::Validating, BuildPhase::Activating] {
            match progress(checkpoint.progress(phase))? {
                BuildControl::Pause => {
                    return Ok((BuildOutcome::Paused(checkpoint.progress(phase)), drains))
                }
                BuildControl::Cancel => {
                    drop(index);
                    discard(out, &path)?;
                    return Ok((BuildOutcome::Cancelled, drains));
                }
                BuildControl::Continue => {}
            }
            if phase == BuildPhase::Validating {
                if !index.validate_checksum().map_err(index_error)?.is_empty() {
                    return Err(PackError::Corrupt("index checksum failed".into()));
                }
                Index::open_generation(&path, zim)?;
            }
        }
        storage::point(out, "CURRENT", &path)?;
        finish_publication(out, &path)?;
        Ok((BuildOutcome::Ready(meta), drains))
    })
}

pub(super) fn build_lock(out: &Path) -> Result<fs::File, PackError> {
    let lock_path = out.join("build.lock");
    if let Ok(meta) = fs::symlink_metadata(&lock_path) {
        if !meta.is_file() || meta.file_type().is_symlink() {
            return Err(PackError::Corrupt("invalid index build lock".into()));
        }
    }
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(lock_path)?;
    lock.try_lock_exclusive().map_err(|error| {
        if error.raw_os_error() == fs2::lock_contended_error().raw_os_error() {
            PackError::Mismatch("index build already owned".into())
        } else {
            PackError::from(error)
        }
    })?;
    Ok(lock)
}

pub(super) fn capture_preparation_current(
    out: &Path,
) -> Result<Option<std::path::PathBuf>, PackError> {
    match fs::symlink_metadata(out.join("CURRENT")) {
        Ok(_) => storage::generation(out).map(Some),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

/// Native coordinator authority only: cancel an owned preparation without opening its source.
pub(super) fn cancel_owned_preparation(
    out: &Path,
    meta: &IndexMeta,
    before: &Option<std::path::PathBuf>,
) -> Result<(), PackError> {
    guarded(|| {
        let _lock = build_lock(out)?;
        let current = capture_preparation_current(out)?;
        if current != *before {
            return Err(PackError::PublicationUncertain(
                "index publication changed before preparation cleanup".into(),
            ));
        }
        let path = match fs::symlink_metadata(out.join("BUILDING")) {
            Ok(_) => storage::pointed_path(out, "BUILDING")?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(error.into()),
        };
        if current.as_ref() == Some(&path) {
            return Err(PackError::PublicationUncertain(
                "index preparation is already active".into(),
            ));
        }
        match fs::symlink_metadata(&path) {
            Ok(_) => {
                storage::check_generation(&path)?;
                // The owner marker is removed last; an empty/missing directory is retryable.
                if fs::read_dir(&path)?.next().is_some() && storage::read_owner(&path)? != *meta {
                    return Err(PackError::Mismatch(
                        "index preparation owner differs".into(),
                    ));
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        discard(out, &path)
    })
}

fn read_checkpoint(index: &tantivy::Index, meta: &IndexMeta) -> Result<Checkpoint, PackError> {
    let payload = index
        .load_metas()
        .map_err(index_error)?
        .payload
        .ok_or_else(|| PackError::Corrupt("missing index commit cursor".into()))?;
    if payload.len() > 4096 {
        return Err(PackError::Corrupt("oversized index commit cursor".into()));
    }
    let checkpoint: Checkpoint = serde_json::from_str(&payload).map_err(index_error)?;
    if checkpoint.version != 1 || checkpoint.meta != *meta {
        return Err(PackError::Mismatch(
            "index commit source/profile differs".into(),
        ));
    }
    let count = index.reader().map_err(index_error)?.searcher().num_docs();
    if count != checkpoint.articles {
        return Err(PackError::Corrupt(
            "index documents differ from committed cursor".into(),
        ));
    }
    Ok(checkpoint)
}

fn discard(root: &Path, path: &Path) -> Result<(), PackError> {
    // Never remove a generation already activated before a process interruption.
    if root.join("CURRENT").exists() && storage::generation(root)? == path {
        return Err(PackError::Mismatch(
            "cannot discard an activated index generation".into(),
        ));
    }
    match fs::symlink_metadata(path) {
        Ok(_) => {
            storage::check_generation(path)?;
            // Retain the authority marker until every other file is gone, so
            // interrupted/failed cleanup is retryable without reopening Tantivy.
            for entry in fs::read_dir(path)? {
                let entry = entry?;
                if entry.file_name() != storage::OWNER {
                    fs::remove_file(entry.path())?;
                }
            }
            match fs::remove_file(path.join(storage::OWNER)) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(e.into()),
            }
            fs::remove_dir(path)?;
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.into()),
    }
    fs::remove_file(root.join("BUILDING"))?;
    Ok(())
}

fn finish_publication(root: &Path, path: &Path) -> Result<(), PackError> {
    let finish = || -> Result<(), PackError> {
        match fs::remove_file(path.join(storage::OWNER)) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
        fs::remove_file(root.join("BUILDING"))?;
        Ok(())
    };
    finish().map_err(|error| {
        PackError::PublicationUncertain(format!("index activated; cleanup incomplete: {error}"))
    })
}
