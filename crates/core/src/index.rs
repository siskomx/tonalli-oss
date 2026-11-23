//! Immutable full-text generations on app-internal storage, bound to one ZIM.
mod build;
mod directory;
mod html;
pub mod jobs;
pub use html::article_text;
mod query;
pub mod snapshot;
mod storage;
mod writer;
pub use build::{build_controlled, BuildControl, BuildOutcome, BuildPhase, BuildProgress};

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

use serde::{Deserialize, Serialize};
use tantivy::collector::TopDocs;
use tantivy::query::QueryParser;
use tantivy::schema::{Field, Schema, Value, STORED, STRING, TEXT};
use tantivy::{IndexReader, ReloadPolicy, TantivyDocument};

use crate::pack::{PackError, PackSource};
use crate::zim::Zim;

const FORMAT_VERSION: u32 = 2;
// Increment if the app's text analysis policy changes.
const ANALYZER_REVISION: u32 = 1;
const WRITER_BUDGET: usize = 64 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexMeta {
    pub zim_uuid: [u8; 16],
    pub zim_md5: [u8; 16],
    pub format_version: u32,
    pub tantivy_version: String,
    // Legacy metadata parses to an incompatible identity, never to this profile.
    #[serde(default)]
    pub analyzer_revision: u32,
    #[serde(default)]
    pub std_unicode_version: [u8; 3],
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct Hit {
    pub title: String,
    pub url: String,
    pub score: f32,
}

pub struct Index {
    generation: GenerationDescriptor,
    index: tantivy::Index,
    reader: IndexReader,
    title: Field,
    body: Field,
    url: Field,
    source: Arc<dyn PackSource>,
    checksum_pos: u64,
    uuid: [u8; 16],
    md5: [u8; 16],
    removed: AtomicBool,
}

/// Exact immutable generation and analysis identity; never retains a pack lease.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GenerationDescriptor {
    path: PathBuf,
    meta: IndexMeta,
}

impl GenerationDescriptor {
    /// Reopens this generation even if CURRENT has since changed.
    pub fn open(&self, zim: &Zim) -> Result<Index, PackError> {
        guarded(|| {
            storage::check_generation(&self.path)?;
            if storage::read_meta(&self.path)? != self.meta {
                return Err(PackError::Mismatch(
                    "retained index generation changed".into(),
                ));
            }
            Index::open_generation(&self.path, zim)
        })
    }
}

pub fn build(zim: &Zim, out: &Path, threads: usize) -> Result<IndexMeta, PackError> {
    build_with_queue_budget(zim, out, threads, WRITER_BUDGET).map(|(meta, _)| meta)
}

fn build_with_queue_budget(
    zim: &Zim,
    out: &Path,
    threads: usize,
    queue_budget: usize,
) -> Result<(IndexMeta, usize), PackError> {
    let (outcome, drains) = build::controlled(zim, out, threads, queue_budget, |_| {
        Ok(BuildControl::Continue)
    })?;
    match outcome {
        BuildOutcome::Ready(meta) => Ok((meta, drains)),
        _ => Err(PackError::Corrupt("unrequested index build stop".into())),
    }
}

impl Index {
    pub fn generation_descriptor(&self) -> GenerationDescriptor {
        self.generation.clone()
    }
    /// Checks that this index's hit URLs belong to the supplied archive before reading articles.
    /// Also checks the original retained source, preserving media-removal errors.
    pub fn check_zim(&self, zim: &Zim) -> Result<(), PackError> {
        guarded(|| {
            self.check_source()?;
            let actual = identity(zim)?;
            if self.uuid != actual.zim_uuid || self.md5 != actual.zim_md5 {
                return Err(PackError::Mismatch(
                    "index is bound to a different ZIM".into(),
                ));
            }
            Ok(())
        })
    }

    /// `dir` must be an app-owned internal directory, never removable storage.
    pub fn open(dir: &Path, zim: &Zim) -> Result<Self, PackError> {
        guarded(|| Self::open_generation(&storage::generation(dir)?, zim))
    }

    fn open_generation(dir: &Path, zim: &Zim) -> Result<Self, PackError> {
        let meta = storage::read_meta(dir)?;
        if meta != identity(zim)? {
            return Err(PackError::Mismatch(
                "index source or format does not match this ZIM/build".into(),
            ));
        }
        let index = tantivy::Index::open_in_dir(dir).map_err(index_error)?;
        let (expected, title, body, url) = schema();
        if index.schema() != expected {
            return Err(PackError::Mismatch(
                "index schema differs from this build".into(),
            ));
        }
        let reader = index
            .reader_builder()
            .reload_policy(ReloadPolicy::Manual)
            .try_into()
            .map_err(index_error)?;
        let (source, checksum_pos) = zim.index_source();
        Ok(Self {
            generation: GenerationDescriptor {
                path: std::fs::canonicalize(dir)?,
                meta: meta.clone(),
            },
            index,
            reader,
            title,
            body,
            url,
            source,
            checksum_pos,
            uuid: meta.zim_uuid,
            md5: meta.zim_md5,
            removed: AtomicBool::new(false),
        })
    }

    pub fn search(&self, query: &str, limit: usize) -> Result<Vec<Hit>, PackError> {
        guarded(|| {
            self.check_source()?;
            query::validate(query, limit)?;
            let query = query.trim();
            if query.is_empty() || limit == 0 {
                return Ok(Vec::new());
            }
            let parser = QueryParser::for_index(&self.index, vec![self.title, self.body]);
            let query = parser
                .parse_query(query)
                .map_err(|error| PackError::Mismatch(format!("invalid search query: {error}")))?;
            let searcher = self.reader.searcher();
            let docs = searcher
                .search(&query, &TopDocs::with_limit(limit).order_by_score())
                .map_err(index_error)?;
            docs.into_iter()
                .map(|(score, address)| {
                    let document: TantivyDocument = searcher.doc(address).map_err(index_error)?;
                    let title = document
                        .get_first(self.title)
                        .and_then(|value| value.as_str())
                        .ok_or_else(|| PackError::Corrupt("missing stored title".into()))?
                        .to_owned();
                    let url = document
                        .get_first(self.url)
                        .and_then(|value| value.as_str())
                        .ok_or_else(|| PackError::Corrupt("missing stored URL".into()))?
                        .to_owned();
                    Ok(Hit { title, url, score })
                })
                .collect()
        })
    }

    fn check_source(&self) -> Result<(), PackError> {
        if self.removed.load(Ordering::Acquire) {
            return Err(PackError::MediaRemoved);
        }
        let mut md5 = [0; 16];
        let mut filled = 0;
        while filled < md5.len() {
            match self
                .source
                .read_at(self.checksum_pos + filled as u64, &mut md5[filled..])
            {
                Ok(0) => return Err(PackError::Truncated),
                Ok(read) if read <= md5.len() - filled => filled += read,
                Ok(_) => return Err(PackError::Corrupt("invalid source read length".into())),
                Err(PackError::MediaRemoved) => {
                    self.removed.store(true, Ordering::Release);
                    return Err(PackError::MediaRemoved);
                }
                Err(error) => return Err(error),
            }
        }
        if md5 != self.md5 {
            return Err(PackError::Mismatch("source ZIM checksum changed".into()));
        }
        Ok(())
    }
}

fn identity(zim: &Zim) -> Result<IndexMeta, PackError> {
    Ok(IndexMeta {
        zim_uuid: zim.uuid(),
        zim_md5: zim.stored_md5()?,
        format_version: FORMAT_VERSION,
        tantivy_version: tantivy::version_string().into(),
        analyzer_revision: ANALYZER_REVISION,
        std_unicode_version: {
            let (major, minor, patch) = char::UNICODE_VERSION;
            [major, minor, patch]
        },
    })
}

fn schema() -> (Schema, Field, Field, Field) {
    let mut builder = Schema::builder();
    let title = builder.add_text_field("title", TEXT | STORED);
    let body = builder.add_text_field("body", TEXT);
    let url = builder.add_text_field("url", STRING | STORED);
    (builder.build(), title, body, url)
}

fn index_error(error: impl std::fmt::Display) -> PackError {
    PackError::Corrupt(error.to_string())
}
fn guarded<T>(action: impl FnOnce() -> Result<T, PackError>) -> Result<T, PackError> {
    catch_unwind(AssertUnwindSafe(action))
        .unwrap_or_else(|_| Err(PackError::Corrupt("index operation panicked".into())))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pack::PreadSource;

    #[test]
    fn repeated_queue_drains_keep_every_canonical_document_searchable() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../docs/spike/packs/wikipedia_en_knots_maxi_2026-10.zim");
        let zim = Zim::open(Arc::new(
            PreadSource::open(&path).expect("download Task 1 fixture"),
        ))
        .unwrap();
        let expected = zim
            .canonical_article_entries()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        let root = tempfile::tempdir().unwrap();
        let (_, drains) = build_with_queue_budget(&zim, root.path(), 2, 256 * 1024).unwrap();
        assert!(drains > 1, "exercise more than one committed byte boundary");
        let index = Index::open(root.path(), &zim).unwrap();
        assert_eq!(index.reader.searcher().num_docs(), expected.len() as u64);
        for article in expected {
            let hits = index
                .search(
                    &format!(
                        "url:\"{}\"",
                        article.url.replace('\\', "\\\\").replace('"', "\\\"")
                    ),
                    1,
                )
                .unwrap();
            assert_eq!(hits[0].url, article.url);
        }
    }

    #[test]
    fn interrupted_byte_drain_resumes_its_exact_committed_docset() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../docs/spike/packs/wikipedia_en_knots_maxi_2026-10.zim");
        let zim = Zim::open(Arc::new(PreadSource::open(&path).unwrap())).unwrap();
        let expected = zim
            .canonical_article_entries()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        let root = tempfile::tempdir().unwrap();
        let mut cursor = 0;
        let result = build::controlled(&zim, root.path(), 1, 256 * 1024, |p| {
            if p.phase == BuildPhase::Indexing
                && p.committed_articles > 0
                && p.processed_articles > p.committed_articles
            {
                cursor = p.committed_articles;
                return Err(PackError::Io(
                    "interrupted after committed byte drain".into(),
                ));
            }
            Ok(BuildControl::Continue)
        });
        assert!(matches!(result, Err(PackError::Io(_))));
        assert!(cursor > 0);
        let mut first = None;
        build::controlled(&zim, root.path(), 1, 256 * 1024, |p| {
            if p.phase == BuildPhase::Indexing {
                first.get_or_insert(p.processed_articles);
            }
            Ok(BuildControl::Continue)
        })
        .unwrap();
        assert_eq!(first, Some(cursor));
        let index = Index::open(root.path(), &zim).unwrap();
        assert_eq!(index.reader.searcher().num_docs(), expected.len() as u64);
        for article in expected {
            assert_eq!(
                index
                    .search(
                        &format!(
                            "url:\"{}\"",
                            article.url.replace('\\', "\\\\").replace('"', "\\\"")
                        ),
                        1
                    )
                    .unwrap()[0]
                    .url,
                article.url
            );
        }
    }
}
