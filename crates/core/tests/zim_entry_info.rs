use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;

use tonalli_core::pack::{PackError, PackSource, PreadSource};
use tonalli_core::zim::{EntryInfo, Zim};

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/articles")
        .join(name)
}

fn benign() -> Zim {
    Zim::open(Arc::new(
        PreadSource::open(&fixture("benign-v1.zim")).unwrap(),
    ))
    .unwrap()
}

#[test]
fn entry_info_returns_canonical_metadata_for_direct_and_unqualified_paths() {
    let zim = benign();
    for path in ["C/en/index.html", "en/index.html"] {
        assert_eq!(
            zim.entry_info(path).unwrap(),
            EntryInfo {
                title: "English fixture".into(),
                url: "C/en/index.html".into(),
                mime_type: "text/html".into(),
                redirected: false,
            }
        );
    }
}

#[test]
fn entry_info_resolves_actual_writer_redirect_chain() {
    let zim = benign();
    for path in ["C/redirect/start.html", "C/redirect/chain.html"] {
        let info = zim.entry_info(path).unwrap();
        assert_eq!(info.title, "English fixture");
        assert_eq!(info.url, "C/en/index.html");
        assert_eq!(info.mime_type, "text/html");
        assert!(info.redirected);
    }
}

#[test]
fn resource_mime_and_spanish_unicode_metadata_are_preserved() {
    let zim = benign();
    for (path, mime) in [
        ("C/assets/style.css", "text/css"),
        ("C/assets/pixel.png", "image/png"),
        ("C/assets/body-probe.bin", "application/octet-stream"),
    ] {
        let info = zim.entry_info(path).unwrap();
        assert_eq!(info.mime_type, mime);
        assert!(!info.redirected);
        let original = std::fs::read(fixture("source/benign").join(&path[2..])).unwrap();
        assert_eq!(zim.article(path).unwrap().bytes, original);
    }
    let info = zim.entry_info("C/es/Niñez_y_educación.html").unwrap();
    assert_eq!(info.title, "Niñez y educación");
    assert_eq!(info.url, "C/es/Niñez_y_educación.html");
    assert_eq!(info.mime_type, "text/html");
}

struct BlockedBody {
    source: PreadSource,
    blocked: AtomicBool,
    hits: AtomicUsize,
    begin: u64,
    end: u64,
}
impl PackSource for BlockedBody {
    fn len(&self) -> u64 {
        self.source.len()
    }
    fn read_at(&self, off: u64, out: &mut [u8]) -> Result<usize, PackError> {
        if self.blocked.load(Ordering::SeqCst)
            && off < self.end
            && off.saturating_add(out.len() as u64) > self.begin
        {
            self.hits.fetch_add(1, Ordering::SeqCst);
            return Err(PackError::MediaRemoved);
        }
        self.source.read_at(off, out)
    }
}

#[test]
fn redirect_metadata_never_reads_the_resolved_target_blob() {
    let manifest: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(fixture("manifest.json")).unwrap()).unwrap();
    let probe = &manifest["packs"][0]["body_probe"];
    let begin = probe["blocked_offset"].as_u64().unwrap();
    let source = Arc::new(BlockedBody {
        source: PreadSource::open(&fixture("benign-v1.zim")).unwrap(),
        blocked: AtomicBool::new(false),
        hits: AtomicUsize::new(0),
        begin,
        end: begin + probe["blocked_bytes"].as_u64().unwrap(),
    });
    let zim = Zim::open(source.clone()).unwrap();
    source.blocked.store(true, Ordering::SeqCst);
    for path in ["C/redirect/body-probe", "C/assets/body-probe.bin"] {
        let info = zim.entry_info(path).unwrap();
        assert_eq!(info.url, "C/assets/body-probe.bin");
        assert_eq!(info.mime_type, "application/octet-stream");
        assert_eq!(info.redirected, path.starts_with("C/redirect/"));
    }
    assert_eq!(source.hits.load(Ordering::SeqCst), 0);
    assert!(matches!(
        zim.article("C/redirect/body-probe"),
        Err(PackError::MediaRemoved)
    ));
    assert!(source.hits.load(Ordering::SeqCst) > 0);
}

#[test]
fn entry_info_missing_path_returns_not_found() {
    assert!(matches!(
        benign().entry_info("C/not-present"),
        Err(PackError::NotFound(_))
    ));
}

#[test]
fn writer_fixtures_have_valid_md5_and_hostile_payload_is_unchanged() {
    benign().verify_md5().unwrap();
    let zim = Zim::open(Arc::new(
        PreadSource::open(&fixture("hostile-v1.zim")).unwrap(),
    ))
    .unwrap();
    zim.verify_md5().unwrap();
    for name in [
        "passive", "all", "meta", "top", "popup", "form", "ping", "neutral",
    ] {
        let path = format!("C/hostile/{name}.html");
        let info = zim.entry_info(&path).unwrap();
        assert_eq!(info.mime_type, "text/html");
        assert!(!info.redirected);
        assert_eq!(
            zim.article(&path).unwrap().bytes,
            std::fs::read(fixture(&format!("source/hostile/hostile/{name}.html"))).unwrap()
        );
    }
}
