//! Bounded small-index passage export and optional English embedding comparison.

use std::{
    collections::HashSet,
    fs,
    io::Write,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tonalli_core::{index::Hit, pack::PackError};

#[cfg(feature = "ai")]
mod bge;
mod runner;

#[derive(Debug, PartialEq)]
enum Mode {
    ExportOnly,
    Bge(String),
}

#[derive(Debug)]
struct Config {
    id: String,
    authority: PathBuf,
    manifest: PathBuf,
    questions: PathBuf,
    output: PathBuf,
    mode: Mode,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Questions {
    version: u32,
    retrieval_language: String,
    note: String,
    questions: Vec<Question>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Question {
    id: String,
    retrieval_query_en: String,
    question_en: String,
    question_es: String,
}

#[derive(Debug, Serialize)]
struct Dataset {
    pack_id: String,
    pack_sha256: String,
    index_manifest_sha256: String,
    index_generation: String,
}

#[derive(Debug, Serialize)]
struct Passage {
    bm25_rank: usize,
    title: String,
    url: String,
    article_sha256: String,
    passage_sha256: String,
    source_text_bytes: usize,
    truncated: bool,
    text: String,
}

#[derive(Debug, Serialize)]
struct Ranked {
    index: usize,
    cosine: Option<f64>,
}

fn invalid(message: &str) -> PackError {
    PackError::Mismatch(format!("ai-rerank: {message}"))
}
fn corrupt(message: &str) -> PackError {
    PackError::Corrupt(format!("ai-rerank: {message}"))
}
fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn parse(args: &[String]) -> Result<Config, PackError> {
    let (mut id, mut authority, mut output, mut manifest, mut questions, mut mode) =
        (None, None, None, None, None, None);
    let mut args = args.iter();
    while let Some(flag) = args.next() {
        if flag == "--export-only" {
            if mode.replace(Mode::ExportOnly).is_some() {
                return Err(invalid("select exactly one mode"));
            }
            continue;
        }
        let value = args
            .next()
            .filter(|value| !value.is_empty() && !value.starts_with("--"))
            .ok_or_else(|| invalid("missing option value"))?;
        match flag.as_str() {
            "--id" if id.is_none() && value.len() <= 256 => id = Some(value.clone()),
            "--index-authority" if authority.is_none() => authority = Some(PathBuf::from(value)),
            "--out" if output.is_none() => output = Some(PathBuf::from(value)),
            "--manifest" if manifest.is_none() => manifest = Some(PathBuf::from(value)),
            "--questions" if questions.is_none() => questions = Some(PathBuf::from(value)),
            "--embedding-id" if mode.is_none() && value.len() <= 256 => {
                mode = Some(Mode::Bge(value.clone()))
            }
            _ => return Err(invalid("unknown, duplicate or ambiguous option")),
        }
    }
    Ok(Config {
        id: id.ok_or_else(|| invalid("--id required"))?,
        authority: authority.ok_or_else(|| invalid("--index-authority required"))?,
        output: output.ok_or_else(|| invalid("--out required"))?,
        mode: mode.ok_or_else(|| invalid("explicit mode required"))?,
        manifest: manifest.unwrap_or_else(|| "docs/spike/packs.toml".into()),
        questions: questions.unwrap_or_else(|| "docs/spike/ai-rerank-questions-v1.json".into()),
    })
}

fn validate_questions(questions: &Questions) -> Result<(), PackError> {
    if questions.version != 1
        || questions.retrieval_language != "en"
        || questions.questions.len() != 10
    {
        return Err(invalid(
            "expected ten English retrieval questions, version 1",
        ));
    }
    let mut ids = HashSet::new();
    for question in &questions.questions {
        if question.id.is_empty()
            || question.id.len() > 32
            || !question
                .id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
            || !ids.insert(&question.id)
            || question.retrieval_query_en != question.question_en
            || [&question.question_en, &question.question_es]
                .iter()
                .any(|text| text.trim().is_empty() || text.len() > 2048 || text.contains('\0'))
        {
            return Err(invalid(
                "invalid paired question or manually rewritten retrieval query",
            ));
        }
    }
    Ok(())
}

fn freeze_passage(
    hit: &Hit,
    bm25_rank: usize,
    article: &[u8],
    plain: &str,
) -> Result<Passage, PackError> {
    if !(1..=20).contains(&bm25_rank) || plain.trim().is_empty() {
        return Err(invalid("invalid source rank or empty extracted passage"));
    }
    let mut end = plain.len().min(1024);
    while !plain.is_char_boundary(end) {
        end -= 1;
    }
    let text = plain[..end].to_owned();
    Ok(Passage {
        bm25_rank,
        title: hit.title.clone(),
        url: hit.url.clone(),
        article_sha256: digest(article),
        passage_sha256: digest(text.as_bytes()),
        source_text_bytes: plain.len(),
        truncated: end < plain.len(),
        text,
    })
}

fn export_question(
    directory: &Path,
    dataset: &Dataset,
    question: &Question,
    passages: &[Passage],
    order: &[Ranked],
) -> Result<(), PackError> {
    let mut seen = HashSet::new();
    if passages.len() > 20
        || order.len() != passages.len()
        || order.iter().any(|rank| {
            rank.index >= passages.len()
                || !seen.insert(rank.index)
                || rank
                    .cosine
                    .is_some_and(|score| !score.is_finite() || !(-1.0..=1.0).contains(&score))
        })
    {
        return Err(corrupt("invalid ranking permutation/score"));
    }
    fs::create_dir(directory)?;
    let mut selected = Vec::new();
    for (position, rank) in order.iter().take(3).enumerate() {
        let passage = &passages[rank.index];
        let filename = format!("passage-{}.txt", position + 1);
        write_new(&directory.join(&filename), passage.text.as_bytes())?;
        selected.push(serde_json::json!({ "selected_rank":position+1, "bm25_rank":passage.bm25_rank,
            "title":passage.title,"url":passage.url,"article_sha256":passage.article_sha256,
            "passage_sha256":passage.passage_sha256,"file":filename,"bytes":passage.text.len(),
            "source_text_bytes":passage.source_text_bytes,"truncated":passage.truncated,"input_tokens":null }));
    }
    let metadata = serde_json::json!({"version":1,"dataset":dataset,"question":question,"passages":passages,
        "ranking":order,"selected":selected,"retrieval_language":"en","spanish_retrieval_quality_evaluated":false,
        "judgments":{"bm25_top3_supported":null,"reranked_top3_supported":null,"comment":null}});
    write_json(&directory.join("question.json"), &metadata)
}

fn write_new(path: &Path, bytes: &[u8]) -> Result<(), PackError> {
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}

fn write_json(path: &Path, value: &serde_json::Value) -> Result<(), PackError> {
    write_new(
        path,
        &serde_json::to_vec_pretty(value).map_err(|error| corrupt(&error.to_string()))?,
    )
}

pub fn run(args: &[String]) -> Result<(), PackError> {
    let config = parse(args)?;
    #[cfg(not(feature = "ai"))]
    if matches!(config.mode, Mode::Bge(_)) {
        return Err(invalid("BGE mode requires the ai feature"));
    }
    runner::run(config)
}

#[cfg(test)]
#[path = "rerank/tests.rs"]
mod tests;
