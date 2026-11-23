#[path = "fixtures/pmtiles/mod.rs"]
mod fixture;

use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use fixture::{
    archive, directory, gzip, replace_metadata, set_u64, simple, Source, COMPRESSED_LIMIT, MVT,
};
use tonalli_core::pack::{PackError, PackSource, PreadSource};
use tonalli_core::tiles::TileSource;

fn open(bytes: Vec<u8>) -> Result<TileSource, PackError> {
    TileSource::open(Source::shared(bytes))
}
fn corrupt<T>(result: Result<T, PackError>) {
    assert!(
        matches!(
            result,
            Err(PackError::Corrupt(_)) | Err(PackError::Truncated) | Err(PackError::Mismatch(_))
        ),
        "expected typed malformed-pack error"
    );
}

#[test]
fn tile_zero_is_nonempty_on_authored_fixture() {
    let tiles = open(simple()).unwrap();
    assert_eq!(tiles.tile(0, 0, 0).unwrap().as_deref(), Some(MVT));
    assert_eq!(tiles.tile(1, 0, 0).unwrap(), None);
}

#[test]
fn tile_returns_decompressed_format_bytes() {
    let compressed = gzip(MVT);
    let root = gzip(&directory(&[(0, 1, compressed.len() as u64, 0)]));
    let tiles = open(archive(&root, &[], &compressed, 2, 2)).unwrap();
    assert_eq!(tiles.tile(0, 0, 0).unwrap().as_deref(), Some(MVT));
}

#[test]
fn metadata_is_cached_and_matches_header_and_attribution() {
    let source = Source::shared(simple());
    let tiles = TileSource::open(source.clone()).unwrap();
    let reads = source.calls.load(Ordering::Relaxed);
    let metadata = tiles.metadata();
    assert_eq!(metadata.bounds, [-100.0, 10.0, -99.0, 20.0]);
    assert_eq!(metadata.center, [-99.5, 15.0]);
    assert_eq!((metadata.min_zoom, metadata.max_zoom), (0, 8));
    assert_eq!(metadata.attribution, "OpenStreetMap contributors");
    assert_eq!(source.calls.load(Ordering::Relaxed), reads);
}

#[test]
fn metadata_requires_an_object_and_missing_attribution_is_empty() {
    for metadata in [b"[]".as_slice(), b"null", b"invalid"] {
        corrupt(open(replace_metadata(simple(), metadata)));
    }
    assert_eq!(
        open(replace_metadata(simple(), b"{}"))
            .unwrap()
            .metadata()
            .attribution,
        ""
    );
}

#[test]
fn valid_leaf_lookup_and_missing_tile_work() {
    let leaf = directory(&[(0, 1, MVT.len() as u64, 0)]);
    let root = directory(&[(0, 0, leaf.len() as u64, 0)]);
    let tiles = open(archive(&root, &leaf, MVT, 1, 1)).unwrap();
    assert_eq!(tiles.tile(0, 0, 0).unwrap().as_deref(), Some(MVT));
    assert_eq!(tiles.tile(1, 0, 0).unwrap(), None);
}

#[test]
fn compressed_leaf_lookup_works() {
    let leaf = gzip(&directory(&[(0, 1, MVT.len() as u64, 0)]));
    let root = gzip(&directory(&[(0, 0, leaf.len() as u64, 0)]));
    let tiles = open(archive(&root, &leaf, MVT, 2, 1)).unwrap();
    assert_eq!(tiles.tile(0, 0, 0).unwrap().as_deref(), Some(MVT));
}

#[test]
fn leaf_cycles_return_typed_error_in_bounded_reads() {
    for leaves in [vec![1, 0, 0, 5, 1], vec![1, 0, 0, 5, 6, 1, 0, 0, 5, 1]] {
        let root = directory(&[(0, 0, 5, 0)]);
        let source = Source::shared(archive(&root, &leaves, MVT, 1, 1));
        let tiles = TileSource::open(source.clone()).unwrap();
        corrupt(tiles.tile(0, 0, 0));
        assert!(source.calls.load(Ordering::Relaxed) < 20);
    }
}

#[test]
fn maximum_supported_leaf_depth_works_and_next_depth_fails() {
    for count in [6, 7] {
        let mut leaves = Vec::new();
        for index in 0..count {
            let entry = if index + 1 == count {
                (0, 1, MVT.len() as u64, 0)
            } else {
                (0, 0, 5, ((index + 1) * 5) as u64)
            };
            let encoded = directory(&[entry]);
            assert_eq!(encoded.len(), 5);
            leaves.extend(encoded);
        }
        let tiles = open(archive(&directory(&[(0, 0, 5, 0)]), &leaves, MVT, 1, 1)).unwrap();
        if count == 6 {
            assert_eq!(tiles.tile(0, 0, 0).unwrap().as_deref(), Some(MVT));
        } else {
            corrupt(tiles.tile(0, 0, 0));
        }
    }
}

#[test]
fn invalid_coordinates_return_typed_errors() {
    let tiles = open(simple()).unwrap();
    for (z, x, y) in [(32, 0, 0), (0, 1, 0), (0, 0, 1), (31, u32::MAX, 0)] {
        corrupt(tiles.tile(z, x, y));
    }
}

#[test]
fn truncated_header_and_declared_sections_are_errors() {
    for end in [0, 7, 8, 126] {
        assert!(matches!(
            open(simple()[..end].to_vec()),
            Err(PackError::Truncated)
        ));
    }
    let valid = simple();
    for (offset_at, length_at) in [(8, 16), (24, 32), (40, 48), (56, 64)] {
        let mut bytes = valid.clone();
        set_u64(&mut bytes, offset_at, valid.len() as u64);
        set_u64(&mut bytes, length_at, 1);
        assert!(matches!(open(bytes), Err(PackError::Truncated)));
    }
}

#[test]
fn header_overflow_wrong_magic_version_and_compression_are_errors() {
    for at in [8, 24, 40, 56] {
        let mut bytes = simple();
        set_u64(&mut bytes, at, u64::MAX);
        corrupt(open(bytes));
    }
    for (at, value) in [(0, b'X'), (7, 2), (97, 255), (98, 255)] {
        let mut bytes = simple();
        bytes[at] = value;
        corrupt(open(bytes));
    }
}

#[test]
fn malformed_root_varints_counts_lengths_and_offsets_are_errors() {
    let cases: &[&[u8]] = &[
        &[0],
        &[128],
        &[255; 12],
        &[255, 255, 255, 255, 255, 255, 255, 255, 255, 2],
        &[3, 0, 1, 1, 1],
        &[1, 0, 1, 0, 1],
        &[1, 0, 1, 1, 0],
        &[1, 0, 1, 128, 128, 128, 128, 32, 1],
        &[
            2, 255, 255, 255, 255, 255, 255, 255, 255, 255, 1, 1, 1, 1, 1, 1, 1, 1,
        ],
        &[
            2, 0, 1, 1, 1, 2, 1, 255, 255, 255, 255, 255, 255, 255, 255, 255, 1, 0,
        ],
    ];
    for root in cases {
        corrupt(open(archive(root, &[], MVT, 1, 1)));
    }
}

#[test]
fn corrupt_leaf_and_out_of_section_entries_are_errors() {
    for leaf in [
        vec![128],
        directory(&[(0, 1, MVT.len() as u64, u64::MAX - 1)]),
    ] {
        let root = directory(&[(0, 0, leaf.len() as u64, 0)]);
        let tiles = open(archive(&root, &leaf, MVT, 1, 1)).unwrap();
        corrupt(tiles.tile(0, 0, 0));
    }
    let root = directory(&[(0, 0, 5, 1)]);
    let tiles = open(archive(&root, &[1, 0, 1, 1, 1], MVT, 1, 1)).unwrap();
    corrupt(tiles.tile(0, 0, 0));
    let root = directory(&[(0, 1, MVT.len() as u64 + 1, 0)]);
    corrupt(open(archive(&root, &[], MVT, 1, 1)).and_then(|tiles| tiles.tile(0, 0, 0)));
}

#[test]
fn source_short_reads_are_completed_and_premature_eof_is_truncated() {
    let mut source = Source::new(simple());
    source.chunk = 3;
    let tiles = TileSource::open(Arc::new(source)).unwrap();
    assert_eq!(tiles.tile(0, 0, 0).unwrap().as_deref(), Some(MVT));
    let mut source = Source::new(simple());
    source.chunk = 3;
    source.zero_at = Some(100);
    assert!(matches!(
        TileSource::open(Arc::new(source)),
        Err(PackError::Truncated)
    ));
}

#[test]
fn virtual_eight_gib_archive_uses_u64_offsets_and_bounded_positioned_reads() {
    let mut bytes = simple();
    let at = 8_u64 * 1024 * 1024 * 1024;
    set_u64(&mut bytes, 56, at);
    let mut source = Source::new(bytes);
    source.length = at + MVT.len() as u64;
    source.virtual_tile = Some((at, MVT.to_vec()));
    let source = Arc::new(source);
    let tiles = TileSource::open(source.clone()).unwrap();
    assert_eq!(tiles.tile(0, 0, 0).unwrap().as_deref(), Some(MVT));
    assert!(source.largest_read.load(Ordering::Relaxed) <= 16_384);
}

#[test]
fn source_removal_and_other_typed_errors_are_preserved() {
    for error in [
        PackError::MediaRemoved,
        PackError::Io("sentinel read failure".into()),
    ] {
        let mut source = Source::new(simple());
        source.failure = error.clone();
        source.removed.store(true, Ordering::SeqCst);
        assert!(matches!(TileSource::open(Arc::new(source)),Err(actual) if actual==error));
        let mut source = Source::new(simple());
        source.failure = error.clone();
        let source = Arc::new(source);
        let tiles = TileSource::open(source.clone()).unwrap();
        source.removed.store(true, Ordering::SeqCst);
        assert_eq!(tiles.tile(0, 0, 0), Err(error.clone()));
        assert_eq!(tiles.tile(1, 0, 0), Err(error));
    }
}

#[test]
fn oversized_compressed_tile_and_leaf_are_rejected_before_source_allocation() {
    for run in [0, 1] {
        let size = COMPRESSED_LIMIT + 1;
        let root = directory(&[(0, run, size as u64, 0)]);
        let mut bytes = archive(&root, &[], &[], 1, 1);
        set_u64(&mut bytes, if run == 0 { 48 } else { 64 }, size as u64);
        let mut source = Source::new(bytes);
        source.length += size as u64;
        let source = Arc::new(source);
        let tiles = TileSource::open(source.clone()).unwrap();
        corrupt(tiles.tile(0, 0, 0));
        assert!(source.largest_read.load(Ordering::Relaxed) <= 16_384);
    }
}

#[test]
fn gzip_decoded_tile_and_leaf_budget_is_enforced() {
    let bomb = include_bytes!("fixtures/pmtiles/decoded-over-64m.gz");
    assert!(bomb.len() < COMPRESSED_LIMIT);
    let root = directory(&[(0, 1, bomb.len() as u64, 0)]);
    let tiles = open(archive(&root, &[], bomb, 1, 2)).unwrap();
    corrupt(tiles.tile(0, 0, 0));
    let root = gzip(&directory(&[(0, 0, bomb.len() as u64, 0)]));
    let tiles = open(archive(&root, bomb, MVT, 2, 1)).unwrap();
    corrupt(tiles.tile(0, 0, 0));
}

#[test]
fn gzip_metadata_decoded_budget_is_enforced() {
    let root = gzip(&directory(&[(0, 1, MVT.len() as u64, 0)]));
    let bytes = archive(&root, &[], MVT, 2, 1);
    corrupt(open(replace_metadata(
        bytes,
        include_bytes!("fixtures/pmtiles/decoded-over-64m.gz"),
    )));
}

#[test]
fn exact_compressed_and_decoded_tile_limits_are_accepted() {
    let payload = vec![b'X'; COMPRESSED_LIMIT];
    let root = directory(&[(0, 1, payload.len() as u64, 0)]);
    let tiles = open(archive(&root, &[], &payload, 1, 1)).unwrap();
    assert_eq!(tiles.tile(0, 0, 0).unwrap().unwrap(), payload);
    drop(tiles);
    let compressed = include_bytes!("fixtures/pmtiles/decoded-at-64m.gz");
    let root = directory(&[(0, 1, compressed.len() as u64, 0)]);
    let tiles = open(archive(&root, &[], compressed, 1, 2)).unwrap();
    let decoded = tiles.tile(0, 0, 0).unwrap().unwrap();
    assert_eq!(decoded.len(), 64 * 1024 * 1024);
    assert!(decoded.iter().all(|byte| *byte == b'X'));
}

#[test]
fn oversized_compressed_metadata_is_rejected_before_read() {
    let mut bytes = simple();
    let at = bytes.len() as u64;
    set_u64(&mut bytes, 24, at);
    set_u64(&mut bytes, 32, (COMPRESSED_LIMIT + 1) as u64);
    let mut source = Source::new(bytes);
    source.length += COMPRESSED_LIMIT as u64 + 1;
    let source = Arc::new(source);
    corrupt(TileSource::open(source.clone()));
    assert!(source.largest_read.load(Ordering::Relaxed) <= 16_384);
}

#[test]
fn directory_entry_budget_is_enforced_before_allocating_entries() {
    let compressed = include_bytes!("fixtures/pmtiles/entries-over-limit.gz");
    assert!(compressed.len() < 16_257);
    corrupt(open(archive(compressed, &[], MVT, 2, 1)));
}

#[test]
fn corrupt_gzip_and_metadata_return_typed_errors() {
    let mut compressed = gzip(MVT);
    let end = compressed.len();
    compressed[end - 8] ^= 1;
    let tiles = open(archive(
        &directory(&[(0, 1, end as u64, 0)]),
        &[],
        &compressed,
        1,
        2,
    ))
    .unwrap();
    corrupt(tiles.tile(0, 0, 0));
    let mut bytes = simple();
    let at = u64::from_le_bytes(bytes[24..32].try_into().unwrap()) as usize;
    bytes[at] = b'!';
    corrupt(open(bytes));
}

#[test]
fn concurrent_reads_share_no_seek_cursor() {
    let tiles = Arc::new(open(simple()).unwrap());
    let jobs: Vec<_> = (0..8)
        .map(|_| {
            let tiles = tiles.clone();
            std::thread::spawn(move || {
                for _ in 0..20 {
                    assert_eq!(tiles.tile(0, 0, 0).unwrap().as_deref(), Some(MVT));
                }
            })
        })
        .collect();
    for job in jobs {
        job.join().unwrap();
    }
}

#[test]
#[ignore = "external city packs: spike-cli fetch --id map-mx-mexico-city-z8; spike-cli fetch --id map-us-san-diego-z8; spike-cli fetch --id map-ca-montreal-z8; then cargo test -p tonalli-core --test tiles actual_three_city_extracts_decode -- --ignored"]
fn actual_three_city_extracts_decode() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../docs/spike/packs");
    for (file, x, y, expected) in [
        ("mx-mexico-city-z8.pmtiles", 57, 113, 154_739),
        ("us-san-diego-z8.pmtiles", 44, 103, 54_866),
        ("ca-montreal-z8.pmtiles", 75, 91, 201_938),
    ] {
        let source: Arc<dyn PackSource> = Arc::new(PreadSource::open(&root.join(file)).unwrap());
        let tiles = TileSource::open(source).unwrap();
        let tile = tiles.tile(8, x, y).unwrap().unwrap();
        assert_eq!(tile.len(), expected, "{file}");
        assert_eq!(
            tiles.tile(0, 0, 0).unwrap().unwrap().len(),
            93_230,
            "{file}"
        );
        assert_eq!(
            (tiles.metadata().min_zoom, tiles.metadata().max_zoom),
            (0, 8)
        );
    }
}
