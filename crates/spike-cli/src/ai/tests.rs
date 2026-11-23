use super::measurement::*;
use super::*;
use tonalli_core::ai::{GenerationEnd, GenerationEvent, GenerationStats};

fn args(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).into()).collect()
}

#[test]
fn parser_defaults_to_full_fixed_workload_and_explicit_smoke_is_ineligible() {
    let config = parse(&args(&["--id", "qwen", "--out", "new"])).unwrap();
    assert_eq!(config.id, "qwen");
    assert_eq!(config.manifest, PathBuf::from("docs/spike/packs.toml"));
    assert_eq!(config.output, PathBuf::from("new"));
    assert_eq!(config.seconds, 600);
    assert!(!config.smoke);
    let config = parse(&args(&[
        "--id",
        "qwen",
        "--out",
        "new",
        "--smoke-seconds",
        "30",
    ]))
    .unwrap();
    assert_eq!(config.seconds, 30);
    assert!(config.smoke);
}

#[test]
fn parser_rejects_missing_duplicate_unknown_and_unbounded_options() {
    for options in [
        vec![],
        vec!["--id", "qwen"],
        vec!["--id", "qwen", "--out"],
        vec!["--id", "qwen", "--out", "new", "--id", "again"],
        vec!["--id", "qwen", "--out", "new", "--seconds", "600"],
        vec!["--id", "qwen", "--out", "new", "--smoke-seconds", "0"],
        vec!["--id", "qwen", "--out", "new", "--smoke-seconds", "61"],
        vec![
            "--id",
            "qwen",
            "--out",
            "new",
            "--smoke-seconds",
            "18446744073709551616",
        ],
    ] {
        assert!(parse(&args(&options)).is_err(), "{options:?}");
    }
}

fn stats(tokens: u32) -> GenerationStats {
    GenerationStats {
        input_tokens: 2000,
        output_tokens: tokens,
        first_token_us: Some(500),
        total_us: 1000,
        ..GenerationStats::default()
    }
}

fn terminal(id: u64, tokens: u32) -> GenerationEvent {
    GenerationEvent::Finished {
        id,
        reason: GenerationEnd::Limit,
        stats: stats(tokens),
        error: None,
    }
}

#[test]
fn collector_keeps_actual_tokens_distinct_from_utf8_chunk_counts() {
    let mut collector = Collector::default();
    collector
        .observe(GenerationEvent::Started { id: 7 }, 1)
        .unwrap();
    collector
        .observe(
            GenerationEvent::Text {
                id: 7,
                text: "HolaðŸŒŽ".into(),
            },
            800,
        )
        .unwrap();
    collector
        .observe(
            GenerationEvent::Text {
                id: 7,
                text: "ä¸–ç•Œ".into(),
            },
            900,
        )
        .unwrap();
    collector.observe(terminal(7, 5), 1000).unwrap();
    let request = collector.request.unwrap();
    assert_eq!(request.stats.output_tokens, 5);
    assert_eq!(request.chunks, 2);
    assert_eq!(request.bytes, "HolaðŸŒŽä¸–ç•Œ".len() as u64);
    assert_eq!(request.first_receipt_us, Some(800));
}

#[test]
fn collector_rejects_out_of_order_cross_job_duplicate_and_invalid_stats() {
    let mut collector = Collector::default();
    assert!(collector.observe(terminal(7, 5), 1000).is_err());
    collector
        .observe(GenerationEvent::Started { id: 7 }, 1)
        .unwrap();
    assert!(collector
        .observe(GenerationEvent::Started { id: 7 }, 2)
        .is_err());
    assert!(collector
        .observe(
            GenerationEvent::Text {
                id: 8,
                text: "wrong".into()
            },
            3
        )
        .is_err());
    assert!(collector.observe(terminal(7, 129), 1000).is_err());
    collector.observe(terminal(7, 5), 1000).unwrap();
    assert!(collector.observe(terminal(7, 5), 1001).is_err());
    assert!(collector
        .observe(
            GenerationEvent::Text {
                id: 7,
                text: "late".into()
            },
            1002
        )
        .is_err());
}

fn series() -> (Vec<Request>, Vec<Snapshot>) {
    let requests = (0..60)
        .map(|index| Request {
            id: index + 1,
            start_us: index * 10_000_000,
            end_us: (index + 1) * 10_000_000,
            first_receipt_us: Some(1_000_000),
            chunks: 20,
            bytes: 80,
            reason: GenerationEnd::Limit,
            stats: stats(100),
            error: None,
        })
        .collect();
    let snapshots = (1..=20)
        .map(|index| Snapshot {
            scheduled_us: index * 30_000_000,
            observed_us: index * 30_000_000 + 100,
            latest_request_id: Some(index * 3),
        })
        .collect();
    (requests, snapshots)
}

#[test]
fn complete_series_uses_weighted_request_wall_tps_and_smoke_never_passes_gate() {
    let (mut requests, snapshots) = series();
    requests[0].stats.output_tokens = 50;
    requests[1].stats.output_tokens = 100;
    let assessment = assess(&requests, &snapshots, 600_000_100, false);
    assert!(assessment.eligible && assessment.passed, "{assessment:?}");
    assert!(
        (assessment.first_minute_weighted_wall_tokens_s.unwrap() - 550.0 / 60.0).abs() < 0.0001
    );
    assert!(!assess(&requests, &snapshots, 600_000_100, true).eligible);
    assert!(!assess(&requests, &snapshots, 600_000_100, true).passed);
}

#[test]
fn every_sustained_request_must_meet_absolute_and_relative_floors() {
    let (mut requests, snapshots) = series();
    requests[30].stats.output_tokens = 49;
    assert!(!assess(&requests, &snapshots, 600_000_100, false).passed);
    let (mut requests, snapshots) = series();
    for request in &mut requests[..6] {
        request.stats.output_tokens = 128;
    }
    requests[30].stats.output_tokens = 60;
    assert!(!assess(&requests, &snapshots, 600_000_100, false).passed);
}

#[test]
fn missing_baseline_coverage_or_qualified_stats_cannot_pass() {
    let (mut requests, mut snapshots) = series();
    assert!(!assess(&requests, &snapshots, 599_999_999, false).passed);
    snapshots.remove(5);
    assert!(!assess(&requests, &snapshots, 600_000_100, false).passed);
    let (_, snapshots) = series();
    requests.retain(|request| request.end_us > 60_000_000);
    assert!(!assess(&requests, &snapshots, 600_000_100, false).passed);
    let (mut requests, snapshots) = series();
    requests[10].stats.input_tokens = 1999;
    assert!(!assess(&requests, &snapshots, 600_000_100, false).passed);
    requests[10].stats.input_tokens = 2000;
    requests[10].first_receipt_us = Some(10_000_000);
    assert!(!assess(&requests, &snapshots, 600_000_100, false).passed);
}

#[test]
fn calibration_only_accepts_actual_rendered_count_and_rejects_nonprogress() {
    assert_eq!(calibration_next(1000, 1012).unwrap(), Some(1988));
    assert_eq!(calibration_next(1988, 2000).unwrap(), None);
    assert_eq!(calibration_next(2000, 2048).unwrap(), Some(1952));
    for (repeats, observed) in [
        (0, 2000),
        (2001, 2000),
        (1000, 0),
        (1000, u32::MAX),
        (2000, 1999),
    ] {
        assert!(calibration_next(repeats, observed).is_err());
    }
}

#[test]
fn invalid_checksum_and_existing_output_preserve_input_and_old_results() {
    use crate::manifest::{Manifest, Pack};
    let temp = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(temp.path().join("packs/ai")).unwrap();
    let model = temp.path().join("packs/ai/corrupt.gguf");
    let original = b"not model weights";
    std::fs::write(&model, original).unwrap();
    let manifest = temp.path().join("manifest.toml");
    Manifest {
        manifest_version: 1,
        extra: toml::Table::new(),
        packs: vec![Pack {
            id: "fixture".into(),
            kind: "gguf".into(),
            version: "test".into(),
            url: None,
            bytes: original.len() as u64,
            sha256: "0".repeat(64),
            path: "packs/ai/corrupt.gguf".into(),
            deferred: false,
            extra: [("role".into(), toml::Value::String("generation".into()))]
                .into_iter()
                .collect(),
        }],
    }
    .save(&manifest)
    .unwrap();
    let output = temp.path().join("result");
    let command = vec![
        "--id".into(),
        "fixture".into(),
        "--out".into(),
        output.to_string_lossy().into_owned(),
        "--manifest".into(),
        manifest.to_string_lossy().into_owned(),
    ];
    assert_eq!(run(&command), Err(PackError::Checksum));
    assert_eq!(std::fs::read(&model).unwrap(), original);
    let summary: serde_json::Value =
        serde_json::from_slice(&std::fs::read(output.join("summary.json")).unwrap()).unwrap();
    assert_eq!(summary["status"], "failed");
    assert!(!output.join("prompt.txt").exists());
    let before = std::fs::read(output.join("summary.json")).unwrap();
    assert!(run(&command).is_err());
    assert_eq!(std::fs::read(output.join("summary.json")).unwrap(), before);
}

#[test]
fn staging_partial_filename_is_not_an_activated_model_entry() {
    let mut pack = crate::manifest::Pack {
        id: "fixture".into(),
        kind: "gguf".into(),
        version: "test".into(),
        url: None,
        bytes: 17,
        sha256: "0".repeat(64),
        path: "packs/ai/model.gguf.part".into(),
        deferred: false,
        extra: [("role".into(), toml::Value::String("generation".into()))]
            .into_iter()
            .collect(),
    };
    assert!(super::model::validate_pack(&pack, tonalli_core::ai::ModelKind::Generation).is_err());
    pack.path = "packs/ai/model.gguf".into();
    super::model::validate_pack(&pack, tonalli_core::ai::ModelKind::Generation).unwrap();
    pack.kind = "zim".into();
    assert!(super::model::validate_pack(&pack, tonalli_core::ai::ModelKind::Generation).is_err());
}

#[test]
fn embedding_and_unclassified_entries_are_not_generation_benchmarks() {
    let mut pack = crate::manifest::Pack {
        id: "fixture".into(),
        kind: "gguf".into(),
        version: "test".into(),
        url: None,
        bytes: 17,
        sha256: "0".repeat(64),
        path: "packs/ai/model.gguf".into(),
        deferred: false,
        extra: toml::Table::new(),
    };
    let missing = super::model::validate_pack(&pack, tonalli_core::ai::ModelKind::Generation);
    pack.extra.insert("role".into(), "embedding".into());
    let embedding = super::model::validate_pack(&pack, tonalli_core::ai::ModelKind::Generation);
    assert!(
        missing.is_err() && embedding.is_err(),
        "missing={missing:?}, embedding={embedding:?}"
    );
    pack.extra.insert("role".into(), "generation".into());
    super::model::validate_pack(&pack, tonalli_core::ai::ModelKind::Generation).unwrap();
}
