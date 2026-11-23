pub use tonalli_core::{download, pack};
#[allow(dead_code)]
#[path = "fixtures/pmtiles/mod.rs"]
mod fixture;
#[path = "../src/country_maps/ranges.rs"]
mod ranges;

#[test]
fn imported_copy_survives_restart_then_removal_preserves_original() {
    let root = tempfile::tempdir().unwrap();
    let original = root.path().join("original.pmtiles");
    let data = fixture::simple();
    std::fs::write(&original, &data).unwrap();
    let maps = tonalli_core::country_maps::CountryMaps::open(root.path().join("maps")).unwrap();
    let row = maps
        .import(
            std::sync::Arc::new(tonalli_core::pack::PreadSource::open(&original).unwrap()),
            "Test map".into(),
        )
        .unwrap();
    let copy = maps.imported_path(&row).unwrap();
    let reopened = tonalli_core::country_maps::CountryMaps::open(root.path().join("maps")).unwrap();
    assert_eq!(reopened.imports().unwrap()[0].id, row.id);
    assert_eq!(std::fs::read(&copy).unwrap(), data);
    reopened.remove_import(&row.id).unwrap();
    assert!(reopened.imports().unwrap().is_empty());
    assert!(!copy.exists(), "removed copy still occupies storage");
    assert_eq!(std::fs::read(original).unwrap(), data);
}

#[test]
fn map_import_rejects_truncation_without_publishing() {
    let root = tempfile::tempdir().unwrap();
    let maps = tonalli_core::country_maps::CountryMaps::open(root.path().join("maps")).unwrap();
    let file = root.path().join("truncated.pmtiles");
    std::fs::write(&file, b"PMTiles\x03").unwrap();
    let source = std::sync::Arc::new(tonalli_core::pack::PreadSource::open(&file).unwrap());
    assert!(maps.import(source, "Synthetic map".into()).is_err());
    assert!(!root.path().join("maps/imports.json").exists());
    assert_eq!(std::fs::read(file).unwrap(), b"PMTiles\x03");
}

use tonalli_core::country_maps::CountryMaps;
#[test]
fn stale_native_stop_cannot_pause_a_new_country_attempt() {
    let root = tempfile::tempdir().unwrap();
    let maps = CountryMaps::open(root.path().to_owned()).unwrap();
    let old = maps.prepare("mx", 8, true).unwrap();
    maps.control("mx", true).unwrap();
    let current = maps.prepare("mx", 8, true).unwrap();
    assert!(maps.pause_ticket(&old).is_err());
    assert_eq!(maps.list().unwrap()[0].state, "queued");
    maps.pause_ticket(&current).unwrap();
    assert_eq!(maps.list().unwrap()[0].state, "paused");
}
#[test]
fn native_map_admission_is_single_owner_and_cancel_fences_stale_dispatch() {
    let root = tempfile::tempdir().unwrap();
    let maps = CountryMaps::open(root.path().to_owned()).unwrap();
    assert!(maps.prepare("../notes", 8, true).is_err());
    assert!(maps.prepare("mx", 8, false).is_err());
    let ticket = maps.prepare("mx", 8, true).unwrap();
    assert!(maps.prepare("ca", 8, true).is_err());
    maps.control("mx", true).unwrap();
    assert!(maps.claim(ticket).is_err());
    let ticket = maps.prepare("ca", 8, true).unwrap();
    let run = maps.claim(ticket.clone()).unwrap();
    assert!(maps.claim(ticket).is_err());
    assert!(maps.reconcile().is_err());
    maps.control("ca", false).unwrap();
    assert_eq!(maps.list().unwrap()[1].state, "pausing");
    maps.finish(run).unwrap();
    assert_eq!(maps.list().unwrap()[1].state, "paused");
    let reopened = CountryMaps::open(root.path().to_owned()).unwrap();
    assert_eq!(reopened.list().unwrap()[1].state, "paused");
}
