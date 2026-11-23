use std::fs;
use std::path::Path;

use sha2::{Digest, Sha256};
use tonalli_core::pack::PackError;
use tonalli_core::title_index::{FoldedTitleTable, TitleIdentity};

const IDENTITY: TitleIdentity = TitleIdentity {
    zim_uuid: [1; 16],
    zim_md5: [2; 16],
};

fn unicode16_legacy_table() -> Vec<u8> {
    let key = "\u{16ea0}".as_bytes();
    let mut bytes = vec![0_u8; 96];
    bytes[..8].copy_from_slice(b"TONTITL\0");
    bytes[8..12].copy_from_slice(&1_u32.to_le_bytes());
    bytes[12..16].copy_from_slice(&96_u32.to_le_bytes());
    bytes[16..32].copy_from_slice(&IDENTITY.zim_uuid);
    bytes[32..48].copy_from_slice(&IDENTITY.zim_md5);
    bytes[48..56].copy_from_slice(&1_u64.to_le_bytes());
    bytes[56..64].copy_from_slice(&124_u64.to_le_bytes());
    bytes.extend_from_slice(&112_u64.to_le_bytes());
    bytes.extend_from_slice(&124_u64.to_le_bytes());
    bytes.extend_from_slice(&7_u32.to_le_bytes());
    bytes.extend_from_slice(&4_u32.to_le_bytes());
    bytes.extend_from_slice(key);
    let hash = Sha256::digest(&bytes[96..]);
    bytes[64..96].copy_from_slice(&hash);
    bytes
}

#[test]
fn legacy_unicode16_table_must_be_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("legacy.bin");
    fs::write(&path, unicode16_legacy_table()).unwrap();
    assert!(matches!(
        FoldedTitleTable::open(&path, &IDENTITY),
        Err(PackError::Mismatch(_))
    ));
}

#[test]
fn legacy_empty_table_is_mismatch_before_reading_extended_header() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("legacy-empty.bin");
    let mut bytes = unicode16_legacy_table();
    bytes.truncate(104);
    bytes[48..56].copy_from_slice(&0_u64.to_le_bytes());
    bytes[56..64].copy_from_slice(&104_u64.to_le_bytes());
    bytes[96..104].copy_from_slice(&104_u64.to_le_bytes());
    refresh_payload_checksum(&mut bytes);
    fs::write(&path, bytes).unwrap();
    assert!(matches!(
        FoldedTitleTable::open(&path, &IDENTITY),
        Err(PackError::Mismatch(_))
    ));
}

#[test]
fn v2_records_both_unicode_versions_and_folds_new_lowercase_mapping() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("current.bin");
    let table = build(&path, &[(7, "\u{16ea0}")]);
    assert_eq!(table.suggest("\u{16ebb}", 10).unwrap(), [7]);
    let bytes = fs::read(&path).unwrap();
    assert_eq!(u32::from_le_bytes(bytes[8..12].try_into().unwrap()), 2);
    assert_eq!(u32::from_le_bytes(bytes[12..16].try_into().unwrap()), 112);
    assert_eq!(u32::from_le_bytes(bytes[96..100].try_into().unwrap()), 1);
    let (a, b, c) = unicode_normalization::UNICODE_VERSION;
    let (d, e, f) = char::UNICODE_VERSION;
    let expected = [a, b, c, d, e, f]
        .into_iter()
        .flat_map(|v| u16::from(v).to_le_bytes())
        .collect::<Vec<_>>();
    assert_eq!(bytes[100..112], expected);
}

#[test]
fn every_folding_identity_field_is_checked_before_payload_use() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("current.bin");
    drop(build(&path, &[(7, "Alpha")]));
    let original = fs::read(&path).unwrap();
    for offset in [96, 100, 102, 104, 106, 108, 110] {
        let mut bytes = original.clone();
        bytes[offset] ^= 1;
        // Also corrupt payload: identity must be refused before checksum scanning.
        let last = bytes.len() - 1;
        bytes[last] ^= 1;
        fs::write(&path, bytes).unwrap();
        assert!(
            matches!(
                FoldedTitleTable::open(&path, &IDENTITY),
                Err(PackError::Mismatch(_))
            ),
            "field at {offset}"
        );
    }
}

fn build(path: &Path, rows: &[(u32, &str)]) -> FoldedTitleTable {
    FoldedTitleTable::build(
        path,
        IDENTITY,
        rows.iter().map(|(id, title)| Ok((*id, (*title).into()))),
    )
    .unwrap()
}

#[test]
fn folded_prefix_finds_spanish_accents_and_ignores_case() {
    let dir = tempfile::tempdir().unwrap();
    let table = build(
        &dir.path().join("titles.bin"),
        &[(7, "México"), (4, "niños"), (2, "camión"), (9, "Mexicali")],
    );
    assert_eq!(table.suggest("MEXICO", 10).unwrap(), [7]);
    assert_eq!(table.suggest("camion", 10).unwrap(), [2]);
    assert_eq!(table.suggest("NINOS", 10).unwrap(), [4]);
    assert_eq!(table.suggest("MÉXI", 10).unwrap(), [9, 7]);
}

#[test]
fn empty_whitespace_and_limit_are_bounded() {
    let dir = tempfile::tempdir().unwrap();
    let table = build(
        &dir.path().join("titles.bin"),
        &[(2, "Abeja"), (1, "Árbol"), (3, "Avión")],
    );
    assert!(table.suggest("", 10).unwrap().is_empty());
    assert!(table.suggest(" \t\n", 10).unwrap().is_empty());
    assert!(table.suggest("a", 0).unwrap().is_empty());
    assert_eq!(table.suggest("a", 2).unwrap(), [2, 1]);
    assert_eq!(table.suggest("  ARBOL  ", 100).unwrap(), [1]);
    assert!(table.suggest("absent", 10).unwrap().is_empty());
}

#[test]
fn duplicate_keys_have_deterministic_order_and_persistent_bytes() {
    let dir = tempfile::tempdir().unwrap();
    let first = dir.path().join("first.bin");
    let second = dir.path().join("second.bin");
    let rows = [(9, "Árbol"), (2, "arbol"), (7, "ARBOL")];
    let table = build(&first, &rows);
    build(&second, &[(7, "ARBOL"), (9, "Árbol"), (2, "arbol")]);
    assert_eq!(table.suggest("arbol", 10).unwrap(), [2, 7, 9]);
    assert_eq!(fs::read(&first).unwrap(), fs::read(&second).unwrap());
    drop(table);
    assert_eq!(
        FoldedTitleTable::open(&first, &IDENTITY)
            .unwrap()
            .suggest("arbol", 2)
            .unwrap(),
        [2, 7]
    );
}

#[test]
fn non_latin_prefixes_are_preserved() {
    let dir = tempfile::tempdir().unwrap();
    let table = build(
        &dir.path().join("titles.bin"),
        &[(3, "北京"), (8, "北京市"), (6, "Москва"), (4, "東京")],
    );
    assert_eq!(table.suggest("北京", 10).unwrap(), [3, 8]);
    assert_eq!(table.suggest("МОС", 10).unwrap(), [6]);
    assert_eq!(table.suggest("東京", 10).unwrap(), [4]);
}

#[test]
fn empty_table_can_be_reopened() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("titles.bin");
    build(&path, &[]);
    assert!(FoldedTitleTable::open(&path, &IDENTITY)
        .unwrap()
        .suggest("a", 10)
        .unwrap()
        .is_empty());
}

#[test]
fn identity_and_format_version_mismatches_are_refused() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("titles.bin");
    build(&path, &[(1, "Alpha")]);
    for wrong in [
        TitleIdentity {
            zim_uuid: [3; 16],
            ..IDENTITY
        },
        TitleIdentity {
            zim_md5: [3; 16],
            ..IDENTITY
        },
    ] {
        assert!(matches!(
            FoldedTitleTable::open(&path, &wrong),
            Err(PackError::Mismatch(_))
        ));
    }
    let mut bytes = fs::read(&path).unwrap();
    bytes[8..12].copy_from_slice(&u32::MAX.to_le_bytes());
    fs::write(&path, bytes).unwrap();
    assert!(matches!(
        FoldedTitleTable::open(&path, &IDENTITY),
        Err(PackError::Mismatch(_))
    ));
}

#[test]
fn truncated_table_is_a_typed_error() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("titles.bin");
    build(&path, &[(1, "Alpha")]);
    let complete = fs::read(&path).unwrap();
    for length in [0, 20, complete.len() - 1] {
        fs::write(&path, &complete[..length]).unwrap();
        assert!(matches!(
            FoldedTitleTable::open(&path, &IDENTITY),
            Err(PackError::Truncated)
        ));
    }
}

fn refresh_payload_checksum(bytes: &mut [u8]) {
    let header_len = u32::from_le_bytes(bytes[12..16].try_into().unwrap()) as usize;
    let hash = Sha256::digest(&bytes[header_len..]);
    bytes[64..96].copy_from_slice(&hash);
}

#[test]
fn corrupted_payload_is_rejected_by_streaming_checksum() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("titles.bin");
    build(&path, &[(1, "Alpha")]);
    let mut bytes = fs::read(&path).unwrap();
    let last = bytes.len() - 1;
    bytes[last] ^= 1;
    fs::write(&path, bytes).unwrap();
    assert!(matches!(
        FoldedTitleTable::open(&path, &IDENTITY),
        Err(PackError::Checksum)
    ));
}

#[test]
fn corrupt_offset_and_invalid_utf8_never_panic_or_allocate_from_untrusted_length() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("titles.bin");
    build(&path, &[(1, "Alpha"), (2, "Beta"), (3, "Gamma")]);
    let original = fs::read(&path).unwrap();
    let mut bytes = original.clone();
    bytes[120..128].copy_from_slice(&u64::MAX.to_le_bytes());
    refresh_payload_checksum(&mut bytes);
    fs::write(&path, bytes).unwrap();
    let result =
        FoldedTitleTable::open(&path, &IDENTITY).and_then(|table| table.suggest("beta", 10));
    assert!(matches!(result, Err(PackError::Corrupt(_))));
    let mut bytes = original;
    let last = bytes.len() - 1;
    bytes[last] = 0xff;
    refresh_payload_checksum(&mut bytes);
    fs::write(&path, bytes).unwrap();
    let result =
        FoldedTitleTable::open(&path, &IDENTITY).and_then(|table| table.suggest("gamma", 10));
    assert!(matches!(result, Err(PackError::Corrupt(_))));
}

#[test]
fn interrupted_build_preserves_existing_table_and_leaves_no_staging() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("titles.bin");
    let old = build(&path, &[(1, "Alpha")]);
    let before = fs::read(&path).unwrap();
    let entries = [
        Ok((2, "Beta".into())),
        Err(PackError::Io("canceled source iteration".into())),
    ];
    assert!(FoldedTitleTable::build(&path, IDENTITY, entries).is_err());
    assert_eq!(fs::read(&path).unwrap(), before);
    assert_eq!(old.suggest("alpha", 10).unwrap(), [1]);
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
}

#[test]
fn excessive_title_length_is_rejected_without_replacing_table() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("titles.bin");
    build(&path, &[(1, "Alpha")]);
    let before = fs::read(&path).unwrap();
    let result = FoldedTitleTable::build(&path, IDENTITY, [Ok((2, "x".repeat(1_000_000)))]);
    assert!(matches!(result, Err(PackError::Mismatch(_))));
    assert_eq!(fs::read(&path).unwrap(), before);
}

#[test]
fn successful_rebuild_replaces_table_after_closing_reader() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("titles.bin");
    let old = build(&path, &[(1, "Alpha")]);
    drop(old);
    let replacement = build(&path, &[(2, "Beta")]);
    assert_eq!(replacement.suggest("beta", 10).unwrap(), [2]);
    assert_eq!(
        FoldedTitleTable::open(&path, &IDENTITY)
            .unwrap()
            .suggest("beta", 10)
            .unwrap(),
        [2]
    );
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
}

#[cfg(windows)]
#[test]
fn windows_live_reader_prevents_replacement_without_losing_existing_table() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("titles.bin");
    let old = build(&path, &[(1, "Alpha")]);
    let before = fs::read(&path).unwrap();
    let result = FoldedTitleTable::build(&path, IDENTITY, [Ok((2, "Beta".into()))]);
    assert!(matches!(result, Err(PackError::Io(_))));
    assert_eq!(fs::read(&path).unwrap(), before);
    assert_eq!(old.suggest("alpha", 10).unwrap(), [1]);
    drop(old);
    assert_eq!(
        build(&path, &[(2, "Beta")]).suggest("beta", 10).unwrap(),
        [2]
    );
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
}

#[test]
fn malformed_count_and_record_length_are_rejected_before_allocation() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("titles.bin");
    build(&path, &[(1, "Alpha")]);
    let original = fs::read(&path).unwrap();
    let mut bytes = original.clone();
    bytes[48..56].copy_from_slice(&u64::MAX.to_le_bytes());
    fs::write(&path, bytes).unwrap();
    assert!(matches!(
        FoldedTitleTable::open(&path, &IDENTITY),
        Err(PackError::Corrupt(_))
    ));
    let mut bytes = original;
    let record = u64::from_le_bytes(bytes[112..120].try_into().unwrap()) as usize;
    bytes[record + 4..record + 8].copy_from_slice(&u32::MAX.to_le_bytes());
    refresh_payload_checksum(&mut bytes);
    fs::write(&path, bytes).unwrap();
    let result =
        FoldedTitleTable::open(&path, &IDENTITY).and_then(|table| table.suggest("alpha", 10));
    assert!(matches!(result, Err(PackError::Corrupt(_))));
}

#[test]
#[ignore = "explicit synthetic startup-I/O measurement; not a full-pack benchmark"]
fn measure_synthetic_open_scan_and_prefix_query() {
    use std::time::Instant;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("titles.bin");
    let started = Instant::now();
    let table = FoldedTitleTable::build(
        &path,
        IDENTITY,
        (0..20_000).map(|id| Ok((id, format!("Artículo {id:08}")))),
    )
    .unwrap();
    let build_ms = started.elapsed().as_secs_f64() * 1_000.0;
    drop(table);
    let mut open_us = Vec::new();
    for _ in 0..30 {
        let started = Instant::now();
        let table = FoldedTitleTable::open(&path, &IDENTITY).unwrap();
        open_us.push(started.elapsed().as_secs_f64() * 1_000_000.0);
        drop(table);
    }
    let table = FoldedTitleTable::open(&path, &IDENTITY).unwrap();
    let mut suggest_us = Vec::new();
    for _ in 0..30 {
        let started = Instant::now();
        assert_eq!(table.suggest("ARTICULO 000000", 10).unwrap().len(), 10);
        suggest_us.push(started.elapsed().as_secs_f64() * 1_000_000.0);
    }
    open_us.sort_by(f64::total_cmp);
    suggest_us.sort_by(f64::total_cmp);
    println!("synthetic_titles=20000 table_bytes={} build_ms={build_ms:.3} warm_open_p50_us={:.3} warm_open_p95_us={:.3} warm_suggest_p50_us={:.3} warm_suggest_p95_us={:.3}", fs::metadata(path).unwrap().len(), open_us[14], open_us[28], suggest_us[14], suggest_us[28]);
}
