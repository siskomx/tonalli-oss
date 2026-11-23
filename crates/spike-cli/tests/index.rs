use std::{fs, path::PathBuf, process::Command};

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../docs/spike/packs/wikipedia_en_knots_maxi_2026-10.zim")
}

#[test]
fn builds_actual_small_fixture_and_reports_fixed_query_results() {
    let temp = tempfile::tempdir().unwrap();
    let out = temp.path().join("new-index");
    let output = Command::new(env!("CARGO_BIN_EXE_spike-cli"))
        .args(["index", "--zim"])
        .arg(fixture())
        .arg("--out")
        .arg(&out)
        .args(["--threads", "4"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["documents"], 599);
    assert_eq!(report["requested_threads"], 4);
    assert_eq!(report["index_meta"]["format_version"], 2);
    assert!(report["generation_bytes"].as_u64().unwrap() > 0);
    assert!(report["build_elapsed_ms"].is_number());
    let generation = report["generation"].as_str().unwrap();
    assert_eq!(fs::read_to_string(out.join("CURRENT")).unwrap(), generation);
    let queries = report["queries"].as_array().unwrap();
    assert_eq!(
        queries
            .iter()
            .map(|q| q["query"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["solar system", "photosynthesis", "Mexican history"]
    );
    assert!(queries
        .iter()
        .all(|q| q["hits"].as_array().unwrap().len() <= 5));
    // The actual index remains usable for a known fixture article, beyond report shape.
    let zim = tonalli_core::zim::Zim::open(std::sync::Arc::new(
        tonalli_core::pack::PreadSource::open(&fixture()).unwrap(),
    ))
    .unwrap();
    assert!(!tonalli_core::index::Index::open(&out, &zim)
        .unwrap()
        .search("trefoil", 5)
        .unwrap()
        .is_empty());
}

#[test]
fn rejects_existing_output_invalid_arguments_and_missing_source_without_mutation() {
    let temp = tempfile::tempdir().unwrap();
    let old = temp.path().join("old");
    fs::create_dir(&old).unwrap();
    fs::write(old.join("sentinel"), b"historical").unwrap();
    for threads in ["0", "5", "garbage", "4"] {
        let output = Command::new(env!("CARGO_BIN_EXE_spike-cli"))
            .args(["index", "--zim"])
            .arg(fixture())
            .arg("--out")
            .arg(&old)
            .args(["--threads", threads])
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
        assert_eq!(fs::read(old.join("sentinel")).unwrap(), b"historical");
        assert_eq!(fs::read_dir(&old).unwrap().count(), 1);
    }
    let absent = temp.path().join("absent-output");
    for tail in [
        vec!["--threads", "0"],
        vec!["--unknown", "4"],
        vec!["--threads"],
        vec!["--threads", "1", "--threads", "2"],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_spike-cli"))
            .args(["index", "--zim", "missing.zim", "--out"])
            .arg(&absent)
            .args(tail)
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(!absent.exists());
    }
    let output = Command::new(env!("CARGO_BIN_EXE_spike-cli"))
        .args(["index", "--zim", "missing.zim", "--out"])
        .arg(&absent)
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(!absent.exists());
}
