use std::path::PathBuf;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use tonalli_core::pack::{PackError, PackSource, PreadSource};
use tonalli_core::zim::Zim;

fn fixture(es: bool) -> Arc<dyn PackSource> {
    let name = if es {
        "wikipedia_es_climate-change_maxi_2026-10.zim"
    } else {
        "wikipedia_en_knots_maxi_2026-10.zim"
    };
    Arc::new(
        PreadSource::open(
            &PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../docs/spike/packs")
                .join(name),
        )
        .expect("download Task 1 fixtures"),
    )
}

#[test]
fn suggest_finds_known_title_by_prefix() {
    let zim = Zim::open(fixture(false)).unwrap();
    assert!(zim
        .suggest("Trefoil", 10)
        .unwrap()
        .iter()
        .any(|h| h.title == "Trefoil knot"));
}
#[test]
fn suggest_empty_query_returns_empty() {
    let zim = Zim::open(fixture(false)).unwrap();
    assert!(zim.suggest("", 10).unwrap().is_empty());
    assert!(zim.suggest("  ", 10).unwrap().is_empty());
    assert!(zim.suggest("Trefoil", 0).unwrap().is_empty());
}
#[test]
fn suggest_handles_accents_and_cjk_without_panic() {
    let zim = Zim::open(fixture(false)).unwrap();
    zim.suggest("é", 10).unwrap();
    zim.suggest("漢字", 10).unwrap();
}
#[test]
fn suggest_spanish_title_with_ene_and_accents() {
    let zim = Zim::open(fixture(true)).unwrap();
    assert!(zim
        .suggest("Energía renovable", 10)
        .unwrap()
        .iter()
        .any(|h| h.url == "C/Energía_renovable"));
}
#[test]
fn article_follows_redirect() {
    let zim = Zim::open(fixture(false)).unwrap();
    let article = zim.article("C/(2,3)-torus_knot").unwrap();
    assert_eq!(article.url, "C/Trefoil_knot");
    assert_eq!(article.mime_type, "text/html");
    assert!(String::from_utf8_lossy(&article.bytes).contains("Trefoil"));
}
#[test]
fn article_resource_retains_mime() {
    let zim = Zim::open(fixture(false)).unwrap();
    let page = zim.article("C/index").unwrap();
    assert_eq!(page.mime_type, "text/html");
    assert!(!page.bytes.is_empty());
}
struct Removed {
    inner: Arc<dyn PackSource>,
    removed: AtomicBool,
}
impl PackSource for Removed {
    fn len(&self) -> u64 {
        self.inner.len()
    }
    fn read_at(&self, off: u64, buf: &mut [u8]) -> Result<usize, PackError> {
        if self.removed.load(Ordering::SeqCst) {
            return Err(PackError::MediaRemoved);
        }
        self.inner.read_at(off, buf)
    }
}
#[test]
fn suggest_on_removed_media_returns_media_removed_error() {
    let source = Arc::new(Removed {
        inner: fixture(false),
        removed: AtomicBool::new(false),
    });
    let zim = Zim::open(source.clone()).unwrap();
    source.removed.store(true, Ordering::SeqCst);
    assert_eq!(zim.suggest("Trefoil", 10), Err(PackError::MediaRemoved));
    assert_eq!(zim.stored_md5(), Err(PackError::MediaRemoved));
}

#[test]
fn attached_title_fast_paths_preserve_media_removal_until_fresh_reopen() {
    use tonalli_core::pack::ConcatSource;
    let source = Arc::new(Removed {
        inner: fixture(false),
        removed: AtomicBool::new(false),
    });
    let positioned = || {
        Arc::new(
            ConcatSource::from_parts(vec![(
                "wiki.zim".into(),
                source.clone() as Arc<dyn PackSource>,
            )])
            .unwrap(),
        ) as Arc<dyn PackSource>
    };
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("titles.bin");
    let reader = Zim::open(positioned()).unwrap();
    reader.build_title_index(&path).unwrap();
    let reader = reader.with_title_index(&path).unwrap();
    for (prefix, limit) in [
        ("zzzz_no_such_title_987", 10),
        ("", 10),
        ("  ", 10),
        ("trefoil", 0),
    ] {
        assert!(reader.suggest(prefix, limit).unwrap().is_empty());
    }
    source.removed.store(true, Ordering::SeqCst);
    assert_eq!(reader.stored_md5(), Err(PackError::MediaRemoved));
    source.removed.store(false, Ordering::SeqCst);
    // Restored bytes must not revive the already-poisoned owned source.
    for (prefix, limit) in [
        ("zzzz_no_such_title_987", 10),
        ("", 10),
        ("  ", 10),
        ("trefoil", 0),
    ] {
        assert_eq!(
            reader.suggest(prefix, limit),
            Err(PackError::MediaRemoved),
            "prefix={prefix:?} limit={limit}"
        );
    }
    let fresh_source =
        Arc::new(ConcatSource::from_parts(vec![("wiki.zim".into(), fixture(false))]).unwrap());
    let fresh = Zim::open(fresh_source)
        .unwrap()
        .with_title_index(&path)
        .unwrap();
    assert!(fresh
        .suggest("trefoil", 10)
        .unwrap()
        .iter()
        .any(|hit| hit.title == "Trefoil knot"));
    assert!(fresh
        .suggest("zzzz_no_such_title_987", 10)
        .unwrap()
        .is_empty());
}
#[test]
fn truncated_zim_returns_error() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("broken.zim");
    std::fs::write(&p, b"ZIM").unwrap();
    assert!(matches!(
        Zim::open(Arc::new(PreadSource::open(&p).unwrap())),
        Err(PackError::Truncated)
    ));
}
#[test]
fn zim_with_missing_tail_is_truncated() {
    let source = fixture(false);
    let window =
        tonalli_core::pack::WindowSource::new(source.clone(), 0, source.len() - 1).unwrap();
    assert!(matches!(
        Zim::open(Arc::new(window)),
        Err(PackError::Truncated)
    ));
}
#[test]
fn verify_md5_rejects_changed_content() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("changed.zim");
    let source = fixture(false);
    let mut bytes = vec![0; source.len() as usize];
    source.read_at(0, &mut bytes).unwrap();
    bytes[1000] ^= 1;
    std::fs::write(&p, bytes).unwrap();
    let zim = Zim::open(Arc::new(PreadSource::open(&p).unwrap())).unwrap();
    assert_eq!(zim.verify_md5(), Err(PackError::Checksum));
}

#[test]
fn suggest_is_case_insensitive() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("titles.idx");
    let zim = Zim::open(fixture(false)).unwrap();
    zim.build_title_index(&path).unwrap();
    let zim = zim.with_title_index(&path).unwrap();
    assert!(zim
        .suggest("trefoil", 10)
        .unwrap()
        .iter()
        .any(|h| h.title == "Trefoil knot"));
    assert!(matches!(
        zim.build_title_index(&path),
        Err(PackError::Mismatch(_))
    ));
}
#[test]
fn suggest_ignores_accents_es_and_en() {
    let dir = tempfile::tempdir().unwrap();
    for (es, query, url) in [
        (false, "tréfoil", "C/Trefoil_knot"),
        (true, "energia renovable", "C/Energía_renovable"),
    ] {
        let path = dir.path().join(if es { "es.idx" } else { "en.idx" });
        let zim = Zim::open(fixture(es)).unwrap();
        zim.build_title_index(&path).unwrap();
        let zim = zim.with_title_index(&path).unwrap();
        assert!(zim.suggest(query, 10).unwrap().iter().any(|h| h.url == url));
    }
}
#[test]
fn title_table_for_other_zim_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("titles.idx");
    Zim::open(fixture(false))
        .unwrap()
        .build_title_index(&path)
        .unwrap();
    assert!(matches!(
        Zim::open(fixture(true)).unwrap().with_title_index(&path),
        Err(PackError::Mismatch(_))
    ));
}

#[test]
fn legacy_title_search_respects_namespace_order_and_excludes_images() {
    let mut bytes = vec![0; 80];
    bytes.extend_from_slice(b"text/html\0image/png\0\0");
    let mut entries = Vec::new();
    for (ns, title, mime, blob) in [(b'A', "Zulu", 0_u16, 0_u32), (b'I', "Alpha", 1, 1)] {
        entries.push(bytes.len() as u64);
        bytes.extend_from_slice(&mime.to_le_bytes());
        bytes.extend_from_slice(&[0, ns]);
        bytes.extend_from_slice(&0_u32.to_le_bytes());
        bytes.extend_from_slice(&0_u32.to_le_bytes());
        bytes.extend_from_slice(&blob.to_le_bytes());
        bytes.extend_from_slice(title.as_bytes());
        bytes.push(0);
        bytes.extend_from_slice(title.as_bytes());
        bytes.push(0);
    }
    let urls = bytes.len() as u64;
    for n in entries {
        bytes.extend_from_slice(&n.to_le_bytes());
    }
    let titles = bytes.len() as u64;
    bytes.extend_from_slice(&0_u32.to_le_bytes());
    bytes.extend_from_slice(&1_u32.to_le_bytes());
    let clusters = bytes.len() as u64;
    bytes.extend_from_slice(&(clusters + 8).to_le_bytes());
    bytes.push(1);
    for n in [12_u32, 13, 14] {
        bytes.extend_from_slice(&n.to_le_bytes());
    }
    bytes.extend_from_slice(b"ab");
    let checksum = bytes.len() as u64;
    bytes.extend_from_slice(&[0; 16]);
    bytes[0..4].copy_from_slice(&72173914_u32.to_le_bytes());
    bytes[4..6].copy_from_slice(&5_u16.to_le_bytes());
    bytes[24..28].copy_from_slice(&2_u32.to_le_bytes());
    bytes[28..32].copy_from_slice(&1_u32.to_le_bytes());
    for (at, n) in [
        (32, urls),
        (40, titles),
        (48, clusters),
        (56, 80),
        (72, checksum),
    ] {
        bytes[at..at + 8].copy_from_slice(&n.to_le_bytes());
    }
    bytes[68..72].copy_from_slice(&u32::MAX.to_le_bytes());
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("legacy.zim");
    std::fs::write(&path, bytes).unwrap();
    let zim = Zim::open(Arc::new(PreadSource::open(&path).unwrap())).unwrap();
    assert_eq!(zim.suggest("Zulu", 10).unwrap()[0].url, "A/Zulu");
    assert!(zim.suggest("Alpha", 10).unwrap().is_empty());
    let index = dir.path().join("titles.idx");
    zim.build_title_index(&index).unwrap();
    let zim = zim.with_title_index(&index).unwrap();
    assert_eq!(zim.suggest("zulu", 10).unwrap()[0].url, "A/Zulu");
    assert!(zim.suggest("alpha", 10).unwrap().is_empty());
}

#[test]
fn title_listing_cannot_have_more_rows_than_directory_entries() {
    let mut bytes = vec![0; 80];
    bytes.extend_from_slice(b"application/octet-stream\0\0");
    let entry = bytes.len() as u64;
    bytes.extend_from_slice(&[0, 0, 0, b'X']);
    bytes.extend_from_slice(&[0; 12]);
    bytes.extend_from_slice(b"listing/titleOrdered/v1\0\0");
    let urls = bytes.len() as u64;
    bytes.extend_from_slice(&entry.to_le_bytes());
    let pointers = bytes.len() as u64;
    bytes.extend_from_slice(&(pointers + 8).to_le_bytes());
    bytes.push(1);
    for n in [8_u32, 16, 0, 0] {
        bytes.extend_from_slice(&n.to_le_bytes());
    }
    let checksum = bytes.len() as u64;
    bytes.extend_from_slice(&[0; 16]);
    bytes[0..4].copy_from_slice(&72173914_u32.to_le_bytes());
    bytes[4..6].copy_from_slice(&6_u16.to_le_bytes());
    bytes[6..8].copy_from_slice(&3_u16.to_le_bytes());
    bytes[24..28].copy_from_slice(&1_u32.to_le_bytes());
    bytes[28..32].copy_from_slice(&1_u32.to_le_bytes());
    for (at, n) in [(32, urls), (48, pointers), (56, 80), (72, checksum)] {
        bytes[at..at + 8].copy_from_slice(&n.to_le_bytes());
    }
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("listing.zim");
    std::fs::write(&path, bytes).unwrap();
    assert!(matches!(
        Zim::open(Arc::new(PreadSource::open(&path).unwrap())),
        Err(PackError::Corrupt(_))
    ));
}
