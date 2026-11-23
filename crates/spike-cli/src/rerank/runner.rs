use super::*;
use crate::manifest::Pack;
use crate::manifest::{manifest_dir, pack_path, write_atomic, Manifest};
use std::{fs::File, io::Read, sync::Arc, time::Instant};
use tonalli_core::{
    index::Index,
    pack::{verify_sha256, PreadSource},
    zim::Zim,
};

#[derive(Deserialize)]
struct Authority {
    version: u32,
    status: String,
    manifest_sha256: String,
    index_root: PathBuf,
    source_identity: SourceIdentity,
}

#[derive(Deserialize)]
struct SourceIdentity {
    path: PathBuf,
    bytes: String,
}

#[derive(Deserialize)]
struct Snapshot {
    version: u32,
    generation: String,
    source: Source,
    files: Vec<SnapshotFile>,
}

#[derive(Deserialize)]
struct Source {
    uuid: String,
    md5: String,
    bytes: u64,
    expected_sha256: String,
}

#[derive(Deserialize)]
struct SnapshotFile {
    path: String,
    bytes: u64,
    sha256: String,
}

fn component(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 160
        && !matches!(value, "." | "..")
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

fn hex(value: &str, width: usize) -> bool {
    value.len() == width && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn preflight(
    receipt: &[u8],
    snapshot: &[u8],
    pack: &Pack,
) -> Result<(Authority, Snapshot), PackError> {
    if receipt.len() > 1024 * 1024 || snapshot.len() > 1024 * 1024 {
        return Err(invalid("oversized authority JSON"));
    }
    let authority: Authority =
        serde_json::from_slice(receipt).map_err(|error| corrupt(&error.to_string()))?;
    let descriptor: Snapshot =
        serde_json::from_slice(snapshot).map_err(|error| corrupt(&error.to_string()))?;
    if authority.version != 1 || authority.status != "frozen" || descriptor.version != 2 {
        return Err(invalid("unsupported frozen authority"));
    }
    if digest(snapshot) != authority.manifest_sha256 {
        return Err(PackError::Checksum);
    }
    if pack.kind != "zim"
        || pack.bytes > 64 * 1024 * 1024
        || pack.bytes == 0
        || descriptor.source.bytes != pack.bytes
        || descriptor.source.expected_sha256 != pack.sha256
        || authority
            .source_identity
            .bytes
            .parse::<u64>()
            .map_err(|error| corrupt(&error.to_string()))?
            != pack.bytes
        || !hex(&descriptor.source.uuid, 32)
        || !hex(&descriptor.source.md5, 32)
        || !descriptor.generation.starts_with("generation-")
        || !component(&descriptor.generation)
    {
        return Err(invalid(
            "authority must bind the selected small ZIM and generation",
        ));
    }
    if descriptor.files.is_empty() || descriptor.files.len() > 64 {
        return Err(invalid("bounded small-index file table required"));
    }
    let mut names = HashSet::new();
    let mut total = 0_u64;
    for file in &descriptor.files {
        let parts: Vec<_> = file.path.split('/').collect();
        if file.path != "CURRENT"
            && (parts.len() != 2 || parts[0] != descriptor.generation || !component(parts[1]))
        {
            return Err(invalid("invalid frozen index file path"));
        }
        if !names.insert(file.path.as_str()) || !hex(&file.sha256, 64) {
            return Err(invalid("duplicate file or invalid checksum"));
        }
        total = total
            .checked_add(file.bytes)
            .filter(|bytes| *bytes <= 64 * 1024 * 1024)
            .ok_or_else(|| invalid("index exceeds the small-probe byte budget"))?;
    }
    for required in [
        "CURRENT".to_owned(),
        format!("{}/meta.json", descriptor.generation),
        format!("{}/tonalli-meta.json", descriptor.generation),
    ] {
        if !names.contains(required.as_str()) {
            return Err(invalid("incomplete frozen index metadata"));
        }
    }
    Ok((authority, descriptor))
}

fn read_json(path: &Path) -> Result<Vec<u8>, PackError> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > 1024 * 1024 {
        return Err(invalid("invalid bounded JSON file"));
    }
    let mut bytes = Vec::new();
    File::open(path)?
        .take(1024 * 1024 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > 1024 * 1024 {
        return Err(invalid("JSON grew beyond limit"));
    }
    Ok(bytes)
}

fn lease(path: &Path) -> Result<File, PackError> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(invalid("ordinary immutable file required"));
    }
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(1);
    }
    Ok(options.open(path)?)
}

fn micros(start: Instant) -> Result<u64, PackError> {
    u64::try_from(start.elapsed().as_micros()).map_err(|error| corrupt(&error.to_string()))
}

fn save_report(output: &Path, report: &serde_json::Value) -> Result<(), PackError> {
    write_atomic(
        &output.join("run.json"),
        &serde_json::to_vec_pretty(report).map_err(|error| corrupt(&error.to_string()))?,
    )
}

pub(super) fn run(config: Config) -> Result<(), PackError> {
    // Resolve an existing parent before creating any evidence: never add a directory
    // inside an immutable Tantivy generation, including through a junction alias.
    let authority: Authority = serde_json::from_slice(&read_json(&config.authority)?)
        .map_err(|error| corrupt(&error.to_string()))?;
    let index_root = authority.index_root.canonicalize()?;
    let parent = config
        .output
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    if config.output.file_name().is_none() || parent.canonicalize()?.starts_with(index_root) {
        return Err(invalid(
            "output must be a new directory outside the frozen index root",
        ));
    }
    fs::create_dir(&config.output)?;
    let mut report = serde_json::json!({"version":1,"status":"running","platform":"desktop","mode":"export-only","build_debug_assertions":cfg!(debug_assertions),
        "model_loaded":false,"embedding_ranking_performed":false,"quality_assessed":false,
        "retrieval_language":"en","spanish_retrieval_quality_evaluated":false,"passage_byte_budget":1024,
        "retrieval_limit":20,"selected_limit":3});
    if matches!(config.mode, Mode::Bge(_)) {
        report["mode"] = "bge".into();
    }
    save_report(&config.output, &report)?;
    let outcome = execute(&config, &mut report);
    if let Err(error) = &outcome {
        report["status"] = "failed".into();
        report["error"] = error.to_string().into();
    }
    save_report(&config.output, &report)?;
    outcome
}

fn execute(config: &Config, report: &mut serde_json::Value) -> Result<(), PackError> {
    let started = Instant::now();
    let manifest = Manifest::load(&config.manifest)?;
    let pack = manifest
        .packs
        .iter()
        .find(|pack| pack.id == config.id)
        .ok_or_else(|| PackError::NotFound(config.id.clone()))?;
    let raw_authority = read_json(&config.authority)?;
    let raw_snapshot = read_json(
        &config
            .authority
            .parent()
            .unwrap_or(Path::new("."))
            .join("snapshot.json"),
    )?;
    // Validate all source/table bounds before following source or index payload paths.
    let (authority, snapshot) = preflight(&raw_authority, &raw_snapshot, pack)?;
    let source_path = pack_path(manifest_dir(&config.manifest), &pack.path)?;
    if source_path.canonicalize()? != authority.source_identity.path.canonicalize()? {
        return Err(invalid(
            "authority source path differs from selected installed pack",
        ));
    }
    let source = lease(&source_path)?;
    if source.metadata()?.len() != pack.bytes {
        return Err(invalid("source length differs from manifest"));
    }
    verify_sha256(&source_path, &pack.sha256)?; // The admitted source is <=64MiB, never the full corpus.
    let zim = Zim::open(Arc::new(PreadSource::from_file(source.try_clone()?)?))?;
    let hex16 =
        |bytes: [u8; 16]| -> String { bytes.iter().map(|byte| format!("{byte:02x}")).collect() };
    if hex16(zim.uuid()) != snapshot.source.uuid || hex16(zim.stored_md5()?) != snapshot.source.md5
    {
        return Err(invalid("snapshot ZIM UUID/MD5 binding differs"));
    }
    let index_root = authority.index_root.canonicalize()?;
    let mut index_leases = Vec::new();
    for record in &snapshot.files {
        let path = index_root.join(&record.path);
        if !path.canonicalize()?.starts_with(&index_root) {
            return Err(invalid("frozen file escapes index root"));
        }
        let file = lease(&path)?;
        if file.metadata()?.len() != record.bytes {
            return Err(invalid("frozen index file length differs"));
        }
        verify_sha256(&path, &record.sha256)?;
        index_leases.push(file);
    }
    if fs::read_to_string(index_root.join("CURRENT"))?.trim() != snapshot.generation {
        return Err(invalid("frozen generation pointer differs"));
    }
    let index = Index::open(&index_root, &zim)?;
    let raw_questions = read_json(&config.questions)?;
    let questions: Questions =
        serde_json::from_slice(&raw_questions).map_err(|error| corrupt(&error.to_string()))?;
    validate_questions(&questions)?;
    let dataset = Dataset {
        pack_id: pack.id.clone(),
        pack_sha256: pack.sha256.clone(),
        index_manifest_sha256: authority.manifest_sha256,
        index_generation: snapshot.generation,
    };
    report["dataset"] =
        serde_json::to_value(&dataset).map_err(|error| corrupt(&error.to_string()))?;
    report["questions_sha256"] = digest(&raw_questions).into();
    report["setup_us"] = micros(started)?.into();
    write_new(&config.output.join("questions.json"), &raw_questions)?;
    write_new(&config.output.join("authority.json"), &raw_authority)?;
    write_new(&config.output.join("snapshot.json"), &raw_snapshot)?;
    #[cfg(feature = "ai")]
    let series = if let Mode::Bge(id) = &config.mode {
        let pack = manifest
            .packs
            .iter()
            .find(|pack| &pack.id == id)
            .ok_or_else(|| PackError::NotFound(id.clone()))?;
        super::bge::validate_policy(pack)?;
        let policy_path = config.output.join("bge-policy.json");
        let mut policy = serde_json::json!({"model":{"id":pack.id,"version":pack.version,"bytes":pack.bytes,"sha256":pack.sha256},
            "query_prefix":super::bge::PREFIX,"query_prefix_source":"https://huggingface.co/BAAI/bge-small-en-v1.5/raw/main/README.md",
            "documents_modified":false,"verification":"pending","embedding_us":null,"ranking_us":null,
            "rerank_timer_scope":"total adapter and Core embedding-plus-cosine call; internal phases unavailable"});
        write_json(&policy_path, &policy)?;
        let phase = Instant::now();
        let model = super::bge::Series::open(manifest_dir(&config.manifest), pack, |pin| {
            policy["model"] = pin.clone();
            policy["verification"] = "verified".into();
            write_atomic(
                &policy_path,
                &serde_json::to_vec_pretty(&policy).map_err(|error| corrupt(&error.to_string()))?,
            )?;
            report["model"] = pin.clone();
            report["phase"] = "model_load".into();
            save_report(&config.output, report)
        })?;
        report["model_loaded"] = true.into();
        report["model_prepare_us"] = micros(phase)?.into();
        save_report(&config.output, report)?;
        Some(model)
    } else {
        None
    };
    #[cfg(feature = "ai")]
    let mut rerank_csv = if series.is_some() {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(config.output.join("rerank.csv"))?;
        writeln!(
            file,
            "id,embedding_query_sha256,rerank_us,embedding_us,ranking_us,error_present"
        )?;
        Some(file)
    } else {
        None
    };
    let mut csv = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(config.output.join("timings.csv"))?;
    writeln!(csv,"id,retrieval_us,extraction_us,embedding_us,ranking_us,export_us,wall_us,candidates,selected,bm25_top3_supported,reranked_top3_supported")?;
    let mut measured = Vec::new();
    for question in &questions.questions {
        let wall = Instant::now();
        let phase = Instant::now();
        let hits = index.search(&question.retrieval_query_en, 20)?;
        let retrieval_us = micros(phase)?;
        let phase = Instant::now();
        let mut passages = Vec::new();
        let mut raw_hits = Vec::new();
        for (position, hit) in hits.iter().enumerate() {
            if !hit.score.is_finite() {
                return Err(corrupt("nonfinite BM25 score"));
            }
            let article = zim.article(&hit.url)?;
            let plain = tonalli_core::index::article_text(&article.mime_type, &article.bytes)?;
            let resolved = Hit {
                title: article.title,
                url: article.url,
                score: hit.score,
            };
            passages.push(freeze_passage(
                &resolved,
                position + 1,
                &article.bytes,
                &plain,
            )?);
            raw_hits.push(serde_json::json!({"bm25_rank":position+1,"url":hit.url,"title":hit.title,"score":hit.score}));
        }
        let extraction_us = micros(phase)?;
        let order: Vec<_> = (0..passages.len())
            .map(|index| Ranked {
                index,
                cosine: None,
            })
            .collect();
        let rerank_us = None::<u64>;
        #[cfg(feature = "ai")]
        let (order, rerank_us) = if let Some(series) = &series {
            report["current_question"] = question.id.clone().into();
            report["phase"] = "rerank_inputs".into();
            save_report(&config.output, report)?;
            let input = super::bge::query_input(question, &passages)?;
            super::bge::freeze_inputs(
                &config
                    .output
                    .join(format!("{}-bge-inputs.json", question.id)),
                &input,
                &passages,
                series.pin(),
            )?;
            report["phase"] = "rerank".into();
            save_report(&config.output, report)?;
            let phase = Instant::now();
            let ranked = series.rerank(&input);
            let duration = micros(phase)?;
            let file = rerank_csv
                .as_mut()
                .ok_or_else(|| corrupt("missing rerank timing file"))?;
            writeln!(
                file,
                "{},{},{duration},,,{}",
                question.id,
                input.query_sha256,
                ranked.is_err()
            )?;
            file.sync_all()?;
            (ranked?, Some(duration))
        } else {
            (order, rerank_us)
        };
        let phase = Instant::now();
        let folder = config.output.join(&question.id);
        export_question(&folder, &dataset, question, &passages, &order)?;
        write_json(&folder.join("bm25-hits.json"), &serde_json::json!(raw_hits))?;
        let export_us = micros(phase)?;
        let wall_us = micros(wall)?;
        let selected = passages.len().min(3);
        writeln!(
            csv,
            "{},{retrieval_us},{extraction_us},,,{export_us},{wall_us},{},{selected},,",
            question.id,
            passages.len()
        )?;
        csv.sync_all()?;
        let mut row = serde_json::json!({"id":question.id,"retrieval_us":retrieval_us,"extraction_us":extraction_us,
            "export_us":export_us,"wall_us":wall_us,"candidates":passages.len(),"selected":selected});
        if let Some(duration) = rerank_us {
            row["rerank_us"] = duration.into();
        }
        measured.push(row);
    }
    #[cfg(feature = "ai")]
    if let Some(series) = &series {
        series.close()?;
        report["model_closed"] = true.into();
        report["embedding_ranking_performed"] = true.into();
        report["current_question"] = serde_json::Value::Null;
        report["phase"] = "complete".into();
    }
    let mut catalog = Vec::new();
    for relative in [
        "questions.json",
        "authority.json",
        "snapshot.json",
        "timings.csv",
    ]
    .into_iter()
    .map(str::to_owned)
    .chain(questions.questions.iter().flat_map(|question| {
        [
            "question.json",
            "bm25-hits.json",
            "passage-1.txt",
            "passage-2.txt",
            "passage-3.txt",
        ]
        .map(|name| format!("{}/{name}", question.id))
    })) {
        let path = config.output.join(&relative);
        if path.exists() {
            let bytes = fs::read(&path)?;
            catalog.push(
                serde_json::json!({"path":relative,"bytes":bytes.len(),"sha256":digest(&bytes)}),
            );
        }
    }
    #[cfg(feature = "ai")]
    if series.is_some() {
        for relative in ["bge-policy.json".to_owned(), "rerank.csv".to_owned()]
            .into_iter()
            .chain(
                questions
                    .questions
                    .iter()
                    .map(|question| format!("{}-bge-inputs.json", question.id)),
            )
        {
            let bytes = fs::read(config.output.join(&relative))?;
            catalog.push(
                serde_json::json!({"path":relative,"bytes":bytes.len(),"sha256":digest(&bytes)}),
            );
        }
    }
    let catalog =
        serde_json::to_vec_pretty(&catalog).map_err(|error| corrupt(&error.to_string()))?;
    write_new(&config.output.join("export-files.json"), &catalog)?;
    report["export_files_sha256"] = digest(&catalog).into();
    report["measurements"] = measured.into();
    report["questions_completed"] = questions.questions.len().into();
    report["elapsed_us"] = micros(started)?.into();
    report["status"] = "complete".into();
    drop(index);
    drop(index_leases);
    Ok(())
}

#[cfg(test)]
#[path = "runner_tests.rs"]
mod tests;
