use super::*;

fn pack() -> Pack {
    Pack {
        id: "knots".into(),
        kind: "zim".into(),
        version: "test".into(),
        url: None,
        bytes: 14,
        sha256: "a".repeat(64),
        path: "packs/fixture.zim".into(),
        deferred: false,
        extra: toml::Table::new(),
    }
}

fn inputs(snapshot: &serde_json::Value) -> (Vec<u8>, Vec<u8>) {
    let raw = serde_json::to_vec(snapshot).unwrap();
    let receipt = serde_json::json!({"version":1,"status":"frozen","manifest_sha256":digest(&raw),
        "index_root":"does-not-exist-index","source_identity":{"path":"does-not-exist.zim","bytes":snapshot["source"]["bytes"].to_string()}});
    (serde_json::to_vec(&receipt).unwrap(), raw)
}

fn snapshot() -> serde_json::Value {
    serde_json::json!({"version":2,"generation":"generation-fixture",
        "source":{"uuid":"b".repeat(32),"md5":"c".repeat(32),"bytes":14,"expected_sha256":"a".repeat(64)},
        "files":[{"path":"CURRENT","bytes":17,"sha256":"d".repeat(64)},
                 {"path":"generation-fixture/meta.json","bytes":2,"sha256":"e".repeat(64)},
                 {"path":"generation-fixture/tonalli-meta.json","bytes":2,"sha256":"f".repeat(64)}]})
}

#[test]
fn frozen_authority_binds_source_and_descriptor_without_opening_payloads() {
    let (receipt, raw) = inputs(&snapshot());
    let (authority, descriptor) = preflight(&receipt, &raw, &pack()).unwrap();
    assert_eq!(authority.index_root, PathBuf::from("does-not-exist-index"));
    assert_eq!(descriptor.source.bytes, 14);
    let mut changed: serde_json::Value = serde_json::from_slice(&receipt).unwrap();
    changed["manifest_sha256"] = "0".repeat(64).into();
    assert!(matches!(
        preflight(&serde_json::to_vec(&changed).unwrap(), &raw, &pack()),
        Err(PackError::Checksum)
    ));
    let mut wrong = pack();
    wrong.sha256 = "0".repeat(64);
    assert!(matches!(
        preflight(&receipt, &raw, &wrong),
        Err(PackError::Mismatch(_))
    ));
}

#[test]
fn oversized_or_traversing_snapshots_are_rejected_before_payload_io() {
    for mutation in 0..4 {
        let mut value = snapshot();
        match mutation {
            0 => value["source"]["bytes"] = (52_u64 * 1024 * 1024 * 1024).into(),
            1 => value["files"][0]["bytes"] = (65_u64 * 1024 * 1024).into(),
            2 => value["files"][1]["path"] = "../active-full/meta.json".into(),
            _ => value["generation"] = "../active-full".into(),
        }
        let (receipt, raw) = inputs(&value);
        assert!(matches!(
            preflight(&receipt, &raw, &pack()),
            Err(PackError::Mismatch(_))
        ));
    }
}

#[test]
fn output_inside_index_or_generation_is_refused_without_mutating_inputs() {
    let temporary = tempfile::tempdir().unwrap();
    let index = temporary.path().join("index");
    std::fs::create_dir(&index).unwrap();
    std::fs::create_dir(index.join("generation-fixture")).unwrap();
    std::fs::write(index.join("KEEP"), b"installed input").unwrap();
    let receipt = temporary.path().join("authority.json");
    std::fs::write(
        &receipt,
        serde_json::to_vec(
            &serde_json::json!({"version":1,"status":"frozen","manifest_sha256":"a".repeat(64),
        "index_root":index,"source_identity":{"path":"absent.zim","bytes":"14"}}),
        )
        .unwrap(),
    )
    .unwrap();
    let outputs = [
        index.join("evidence"),
        index.join("generation-fixture/evidence"),
    ];
    let results: Vec<_> = outputs
        .iter()
        .map(|output| {
            run(Config {
                id: "knots".into(),
                authority: receipt.clone(),
                manifest: temporary.path().join("missing.toml"),
                questions: temporary.path().join("missing.json"),
                output: output.clone(),
                mode: Mode::ExportOnly,
            })
        })
        .collect();
    assert!(
        results
            .iter()
            .all(|result| matches!(result, Err(PackError::Mismatch(_))))
            && outputs.iter().all(|output| !output.exists()),
        "{results:?}"
    );
    assert_eq!(
        std::fs::read(index.join("KEEP")).unwrap(),
        b"installed input"
    );
    let outside = temporary.path().join("valid-new-output");
    let result = run(Config {
        id: "knots".into(),
        authority: receipt,
        manifest: temporary.path().join("missing.toml"),
        questions: temporary.path().join("missing.json"),
        output: outside.clone(),
        mode: Mode::ExportOnly,
    });
    // A valid new location reaches normal input validation and retains failed-run evidence.
    assert!(matches!(result, Err(PackError::Io(_))));
    assert!(outside.join("run.json").is_file());
}

#[test]
fn linked_output_parent_cannot_alias_the_index() {
    let temporary = tempfile::tempdir().unwrap();
    let index = temporary.path().join("index");
    std::fs::create_dir(&index).unwrap();
    let alias = temporary.path().join("alias");
    #[cfg(windows)]
    {
        let result = std::process::Command::new("rtk")
            .args(["proxy", "cmd", "/c", "mklink", "/J"])
            .arg(&alias)
            .arg(&index)
            .output()
            .unwrap();
        assert!(result.status.success(), "junction fixture creation failed");
    }
    #[cfg(unix)]
    std::os::unix::fs::symlink(&index, &alias).unwrap();
    let receipt = temporary.path().join("authority.json");
    std::fs::write(
        &receipt,
        serde_json::to_vec(
            &serde_json::json!({"version":1,"status":"frozen","manifest_sha256":"a".repeat(64),
        "index_root":index,"source_identity":{"path":"absent.zim","bytes":"14"}}),
        )
        .unwrap(),
    )
    .unwrap();
    let output = alias.join("evidence");
    let result = run(Config {
        id: "knots".into(),
        authority: receipt,
        manifest: temporary.path().join("missing.toml"),
        questions: temporary.path().join("missing.json"),
        output: output.clone(),
        mode: Mode::ExportOnly,
    });
    assert!(matches!(result, Err(PackError::Mismatch(_))), "{result:?}");
    assert!(!output.exists() && !index.join("evidence").exists());
}
