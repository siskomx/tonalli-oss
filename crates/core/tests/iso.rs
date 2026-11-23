use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;

use tonalli_core::iso::{Extent, ExtentSource, IsoImage};
use tonalli_core::pack::{PackError, PackSource, PreadSource};
use tonalli_core::zim::Zim;

const LONG_NAME: &str = "/Niñez_y_educación_larga_fixture.zim";

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/iso")
        .join(name)
}

fn source(name: &str) -> Arc<dyn PackSource> {
    Arc::new(PreadSource::open(&fixture(name)).unwrap())
}

fn original_zim() -> Vec<u8> {
    std::fs::read(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/articles/benign-v1.zim"),
    )
    .unwrap()
}

fn read_all(source: &dyn PackSource) -> Vec<u8> {
    // All callers use the explicitly small tracked fixtures, never a full pack.
    let mut bytes = vec![0; usize::try_from(source.len()).unwrap()];
    source.read_at(0, &mut bytes).unwrap();
    bytes
}

#[test]
fn finds_zim_extent_and_bytes_match_original() {
    let src = source("plain-v1.iso");
    let image = IsoImage::open(src.clone()).unwrap();
    let extents = image.find("/FIXTURE.ZIM").unwrap();
    let expected = original_zim();
    let [Extent::Recorded { offset, len }] = extents.as_slice() else {
        panic!("plain fixture should have one recorded extent");
    };
    assert_eq!(*len, expected.len() as u64);
    let mut actual = vec![0; expected.len()];
    src.read_at(*offset, &mut actual).unwrap();
    assert_eq!(actual, expected);
    assert_eq!(image.find("FIXTURE.ZIM").unwrap(), extents);
}

#[test]
fn opens_zim_through_iso_window() {
    let image = IsoImage::open(source("plain-v1.iso")).unwrap();
    let view = image.open_file("/FIXTURE.ZIM").unwrap();
    let zim = Zim::open(view).unwrap();
    assert!(!zim.suggest("English", 10).unwrap().is_empty());
    assert_eq!(
        zim.entry_info("C/en/index.html").unwrap().title,
        "English fixture"
    );
    assert!(zim.verify_md5().is_ok());
}

#[test]
fn joliet_long_name_found() {
    let image = IsoImage::open(source("joliet-v1.iso")).unwrap();
    assert_eq!(
        read_all(&*image.open_file(LONG_NAME).unwrap()),
        original_zim()
    );
}

#[test]
fn rock_ridge_name_found() {
    let image = IsoImage::open(source("rock-ridge-v1.iso")).unwrap();
    assert_eq!(
        read_all(&*image.open_file(LONG_NAME).unwrap()),
        original_zim()
    );
}

#[test]
fn udf_image_found_without_iso_bridge_fallback() {
    let image = IsoImage::open(source("udf-v1.iso")).unwrap();
    assert_eq!(
        read_all(&*image.open_file(LONG_NAME).unwrap()),
        original_zim()
    );
}

#[test]
fn missing_path_and_non_file_paths_are_typed_errors() {
    let image = IsoImage::open(source("plain-v1.iso")).unwrap();
    assert!(matches!(
        image.find("/missing.zim"),
        Err(PackError::NotFound(_))
    ));
    assert!(image.open_file("/").is_err());
    for path in [
        "../FIXTURE.ZIM",
        "/../FIXTURE.ZIM",
        "a/../FIXTURE.ZIM",
        "a\\b",
        "a\0b",
    ] {
        assert!(
            matches!(image.find(path), Err(PackError::Mismatch(_))),
            "{path:?}"
        );
    }
}

#[test]
fn truncated_and_corrupt_iso_return_errors_without_panics() {
    let valid = std::fs::read(fixture("plain-v1.iso")).unwrap();
    let volume_size_offset = 16 * 2048 + 80;
    let volume_blocks = u32::from_le_bytes(
        valid[volume_size_offset..volume_size_offset + 4]
            .try_into()
            .unwrap(),
    );
    let required_volume_bytes = volume_blocks as usize * 2048;
    assert!(required_volume_bytes > 0 && required_volume_bytes <= valid.len());
    // Cut declared volume bytes, not optional trailing producer padding.
    for length in [0, 31, 16 * 2048, required_volume_bytes - 1] {
        let truncated = Arc::new(BytesSource(valid[..length].to_vec()));
        assert!(matches!(
            IsoImage::open(truncated),
            Err(PackError::Truncated | PackError::Corrupt(_))
        ));
    }
    let mut corrupt = valid;
    corrupt[16 * 2048 + 1..16 * 2048 + 6].copy_from_slice(b"BAD!!");
    assert!(matches!(
        IsoImage::open(Arc::new(BytesSource(corrupt))),
        Err(PackError::Corrupt(_))
    ));
}

#[test]
fn udf_sparing_mapping_handles_large_intermediate_and_bounded_final_address() {
    let bytes = std::fs::read(fixture("sparing-overflow-v1.udf")).unwrap();
    // This synthetic boundary fixture omits VRS. Call the public parser too:
    // facade-only rejection could otherwise bypass the vulnerable address math.
    let limits = libmkiso::udf::Limits {
        max_input_bytes: 64 << 30,
        max_entry_bytes: 64 << 30,
        max_total_bytes: 128 << 30,
        ..Default::default()
    };
    let reader = libmkiso::udf::UdfReader::open(&bytes, limits).unwrap();
    assert_eq!(reader.read_entry(0, 7).unwrap(), b"payload");
    drop(reader);
    assert!(matches!(
        IsoImage::open(Arc::new(BytesSource(bytes))),
        Err(PackError::Corrupt(_) | PackError::Mismatch(_) | PackError::Truncated)
    ));
}

#[test]
fn multi_extent_file_reads_across_every_noncontiguous_boundary() {
    let image = IsoImage::open(source("multi-v1.iso")).unwrap();
    let extents = image.find("/MULTI.BIN").unwrap();
    assert_eq!(extents.len(), 3);
    let view = image.open_file("/MULTI.BIN").unwrap();
    let mut expected = vec![0x11; 2048];
    expected.extend(vec![0x55; 4096]);
    expected.extend(vec![0xe3; 137]);
    assert_eq!(view.len(), expected.len() as u64);
    assert_eq!(read_all(&*view), expected);
    for boundary in [2048, 6144] {
        let mut actual = [0; 32];
        view.read_at(boundary - 16, &mut actual).unwrap();
        assert_eq!(
            &actual,
            &expected[boundary as usize - 16..boundary as usize + 16]
        );
    }
}

#[test]
fn file_payload_signatures_do_not_select_the_volume_format() {
    let original = std::fs::read(fixture("multi-v1.iso")).unwrap();
    let headers: &[&[[u8; 7]]] = &[
        &[*b"\0NSR02\x01"],
        &[*b"\0BEA01\x01", *b"\0NSR02\x01", *b"\0TEA01\x01"],
    ];
    for sequence in headers {
        let mut bytes = original.clone();
        for (index, header) in sequence.iter().enumerate() {
            let start = (24 + index) * 2048;
            bytes[start..start + 7].copy_from_slice(header);
        }
        let image = IsoImage::open(Arc::new(BytesSource(bytes))).unwrap();
        let file = image.open_file("/MULTI.BIN").unwrap();
        let mut actual = [0; 7];
        file.read_at(0, &mut actual).unwrap();
        assert_eq!(&actual, &sequence[0]);
    }
}

struct BytesSource(Vec<u8>);

impl PackSource for BytesSource {
    fn len(&self) -> u64 {
        self.0.len() as u64
    }
    fn read_at(&self, offset: u64, output: &mut [u8]) -> Result<usize, PackError> {
        let start = usize::try_from(offset).map_err(|_| PackError::Truncated)?;
        let end = start
            .checked_add(output.len())
            .ok_or(PackError::Truncated)?;
        output.copy_from_slice(self.0.get(start..end).ok_or(PackError::Truncated)?);
        Ok(output.len())
    }
}

struct Controlled {
    source: Arc<dyn PackSource>,
    removed: AtomicBool,
    max_chunk: usize,
}

impl PackSource for Controlled {
    fn len(&self) -> u64 {
        self.source.len()
    }
    fn read_at(&self, offset: u64, output: &mut [u8]) -> Result<usize, PackError> {
        if self.removed.load(Ordering::SeqCst) {
            return Err(PackError::MediaRemoved);
        }
        let count = output.len().min(self.max_chunk);
        self.source.read_at(offset, &mut output[..count])
    }
}

#[test]
fn parser_and_extent_reader_accept_short_positioned_reads() {
    let src = Arc::new(Controlled {
        source: source("plain-v1.iso"),
        removed: AtomicBool::new(false),
        max_chunk: 7,
    });
    let image = IsoImage::open(src).unwrap();
    assert_eq!(
        read_all(&*image.open_file("/FIXTURE.ZIM").unwrap()),
        original_zim()
    );
}

#[test]
fn media_removal_is_preserved_during_open_and_payload_reads() {
    let src = Arc::new(Controlled {
        source: source("plain-v1.iso"),
        removed: AtomicBool::new(false),
        max_chunk: 4096,
    });
    let image = IsoImage::open(src.clone()).unwrap();
    let file = image.open_file("/FIXTURE.ZIM").unwrap();
    src.removed.store(true, Ordering::SeqCst);
    assert_eq!(file.read_at(0, &mut [0; 8]), Err(PackError::MediaRemoved));
    assert!(matches!(IsoImage::open(src), Err(PackError::MediaRemoved)));
}

#[test]
fn extent_source_fills_zeroes_and_reads_recorded_spans_in_logical_order() {
    let src = Arc::new(BytesSource(b"first---last".to_vec()));
    let view = ExtentSource::new(
        src,
        vec![
            Extent::Recorded { offset: 0, len: 5 },
            Extent::Zero { len: 4 },
            Extent::Recorded { offset: 8, len: 4 },
        ],
    )
    .unwrap();
    assert_eq!(read_all(&view), b"first\0\0\0\0last");
    let mut crossing = [0xff; 8];
    view.read_at(3, &mut crossing).unwrap();
    assert_eq!(&crossing, b"st\0\0\0\0la");
    assert_eq!(view.read_at(13, &mut []), Ok(0));
    assert_eq!(view.read_at(13, &mut [0]), Err(PackError::Truncated));
    assert_eq!(
        view.read_at(u64::MAX, &mut [0; 2]),
        Err(PackError::Truncated)
    );
    assert_eq!(
        ExtentSource::new(Arc::new(BytesSource(vec![])), vec![])
            .unwrap()
            .len(),
        0
    );
}

struct VirtualSource {
    length: u64,
    largest_read: AtomicUsize,
}

impl PackSource for VirtualSource {
    fn len(&self) -> u64 {
        self.length
    }
    fn read_at(&self, offset: u64, output: &mut [u8]) -> Result<usize, PackError> {
        if offset
            .checked_add(output.len() as u64)
            .is_none_or(|end| end > self.length)
        {
            return Err(PackError::Truncated);
        }
        self.largest_read.fetch_max(output.len(), Ordering::SeqCst);
        for (index, byte) in output.iter_mut().enumerate() {
            *byte = ((offset + index as u64) % 251) as u8;
        }
        Ok(output.len())
    }
}

#[test]
fn large_extent_reads_remain_bounded_and_use_64_bit_positions() {
    let src = Arc::new(VirtualSource {
        length: 8 << 30,
        largest_read: AtomicUsize::new(0),
    });
    let view = ExtentSource::new(
        src.clone(),
        vec![Extent::Recorded {
            offset: 1 << 32,
            len: (1 << 32) - 1,
        }],
    )
    .unwrap();
    let mut actual = [0; 64];
    view.read_at(123, &mut actual).unwrap();
    let expected: Vec<_> = (0..64)
        .map(|i| (((1_u64 << 32) + 123 + i) % 251) as u8)
        .collect();
    assert_eq!(actual.as_slice(), expected);
    assert!(src.largest_read.load(Ordering::SeqCst) <= actual.len());
}

#[test]
fn extent_constructor_rejects_source_bounds_and_logical_length_overflow() {
    let src = Arc::new(VirtualSource {
        length: u64::MAX,
        largest_read: AtomicUsize::new(0),
    });
    for extents in [
        vec![Extent::Recorded {
            offset: u64::MAX,
            len: 1,
        }],
        vec![Extent::Zero { len: u64::MAX }, Extent::Zero { len: 1 }],
    ] {
        assert!(matches!(
            ExtentSource::new(src.clone(), extents),
            Err(PackError::Mismatch(_))
        ));
    }
    assert!(matches!(
        ExtentSource::new(
            Arc::new(BytesSource(vec![0; 8])),
            vec![Extent::Recorded { offset: 7, len: 2 }]
        ),
        Err(PackError::Mismatch(_))
    ));
}

#[test]
fn oversized_image_is_rejected_before_any_source_read() {
    let src = Arc::new(VirtualSource {
        length: (64 << 30) + 1,
        largest_read: AtomicUsize::new(0),
    });
    assert!(matches!(
        IsoImage::open(src.clone()),
        Err(PackError::Mismatch(_))
    ));
    assert_eq!(src.largest_read.load(Ordering::SeqCst), 0);
}

struct PaddedImage {
    metadata: Vec<u8>,
    length: u64,
    largest_read: AtomicUsize,
}

impl PackSource for PaddedImage {
    fn len(&self) -> u64 {
        self.length
    }
    fn read_at(&self, offset: u64, output: &mut [u8]) -> Result<usize, PackError> {
        if offset
            .checked_add(output.len() as u64)
            .is_none_or(|end| end > self.length)
        {
            return Err(PackError::Truncated);
        }
        self.largest_read.fetch_max(output.len(), Ordering::SeqCst);
        output.fill(0);
        let start = offset as usize;
        if start < self.metadata.len() {
            let count = output.len().min(self.metadata.len() - start);
            output[..count].copy_from_slice(&self.metadata[start..start + count]);
        }
        Ok(output.len())
    }
}

#[test]
fn directory_metadata_budget_is_enforced_before_large_source_reads() {
    let mut bytes = std::fs::read(fixture("plain-v1.iso")).unwrap();
    let pvd = 16 * 2048;
    let directory =
        u32::from_le_bytes(bytes[pvd + 158..pvd + 162].try_into().unwrap()) as usize * 2048;
    let oversized_directory: u32 = 20 << 20;
    // The virtual image is large enough for this extent, so only the metadata
    // budget, rather than an unrelated source-bound check, should reject it.
    let blocks: u32 = (64 << 20) / 2048;
    bytes[pvd + 80..pvd + 84].copy_from_slice(&blocks.to_le_bytes());
    bytes[pvd + 84..pvd + 88].copy_from_slice(&blocks.to_be_bytes());
    for record in [pvd + 156, directory] {
        bytes[record + 10..record + 14].copy_from_slice(&oversized_directory.to_le_bytes());
        bytes[record + 14..record + 18].copy_from_slice(&oversized_directory.to_be_bytes());
    }
    let src = Arc::new(PaddedImage {
        metadata: bytes,
        length: 64 << 20,
        largest_read: AtomicUsize::new(0),
    });
    let error = IsoImage::open(src.clone())
        .err()
        .expect("metadata budget must reject image");
    assert!(matches!(&error, PackError::Mismatch(_)), "{error:?}");
    assert!(src.largest_read.load(Ordering::SeqCst) <= 64 * 1024);
}

#[test]
#[ignore = "Generate target/iso-fixtures/large-v1.iso with docs/spike/make-iso-fixtures.ps1 first"]
fn file_over_4gb_found_and_read_across_its_section_boundary() {
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/iso-fixtures/large-v1.iso");
    let image = IsoImage::open(Arc::new(PreadSource::open(&path).unwrap())).unwrap();
    let extents = image.find("/LARGE.BIN").unwrap();
    assert!(extents.len() >= 2);
    let file = image.open_file("/LARGE.BIN").unwrap();
    assert!(file.len() > u32::MAX as u64);
    let first_boundary = match extents[0] {
        Extent::Recorded { len, .. } | Extent::Zero { len } => len,
    };
    assert!(first_boundary >= 8 && first_boundary + 8 < file.len());
    let mut across_section = [0; 16];
    file.read_at(first_boundary - 8, &mut across_section)
        .unwrap();
    assert_eq!(&across_section, b"BEFOREEXAFTEREX!");
    // The 4 GiB address is a separate check; it need not be a section boundary.
    let mut bytes = [0; 16];
    file.read_at((1_u64 << 32) - 8, &mut bytes).unwrap();
    assert_eq!(&bytes, b"BEFORE4GAFTER4G!");
}
