mod common;

use std::fs::{self, File, OpenOptions};
use std::sync::Arc;
use std::thread;

use common::FaultySource;
use tempfile::TempDir;
use tonalli_core::pack::{
    ConcatSource, MmapSource, PackError, PackSource, PreadSource, WindowSource,
};

fn file_source(body: &[u8]) -> (TempDir, Arc<dyn PackSource>) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("pack.bin");
    fs::write(&path, body).unwrap();
    let source = PreadSource::open(&path).unwrap();
    (dir, Arc::new(source))
}

fn concat(parts: &[(&str, &[u8])]) -> (TempDir, ConcatSource) {
    let dir = tempfile::tempdir().unwrap();
    let files = parts
        .iter()
        .map(|(name, body)| {
            let path = dir.path().join(name);
            fs::write(&path, body).unwrap();
            ((*name).to_owned(), File::open(path).unwrap())
        })
        .collect();
    let source = ConcatSource::from_files(files).unwrap();
    (dir, source)
}

#[test]
fn mmap_and_pread_return_identical_bytes() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("internal.bin");
    let bytes: Vec<_> = (0..200_000).map(|index| (index % 251) as u8).collect();
    fs::write(&path, &bytes).unwrap();
    let mmap = MmapSource::open(&path).unwrap();
    let pread = PreadSource::open(&path).unwrap();
    assert_eq!(mmap.len(), bytes.len() as u64);
    for (off, length) in [(0, 10), (65_530, 32), (100_000, 40_000), (199_999, 1)] {
        let mut mapped = vec![0; length];
        let mut positioned = vec![0; length];
        assert_eq!(mmap.read_at(off, &mut mapped).unwrap(), length);
        assert_eq!(pread.read_at(off, &mut positioned).unwrap(), length);
        assert_eq!(mapped, positioned);
        assert_eq!(mapped, bytes[off as usize..off as usize + length]);
    }
}

#[test]
fn window_reads_only_its_range() {
    let (_dir, source) = file_source(b"0123456789");
    let window = WindowSource::new(source, 2, 5).unwrap();
    assert_eq!(window.len(), 5);
    let mut buf = [0; 3];
    assert_eq!(window.read_at(1, &mut buf).unwrap(), 3);
    assert_eq!(&buf, b"345");
    assert!(matches!(
        window.read_at(4, &mut buf),
        Err(PackError::Truncated)
    ));
}

#[test]
fn window_past_end_is_error() {
    let (_dir, source) = file_source(b"abc");
    assert!(matches!(
        WindowSource::new(source.clone(), 2, 2),
        Err(PackError::Mismatch(_))
    ));
    assert!(WindowSource::new(source, 3, 0).is_ok());
}

#[test]
fn read_after_truncate_returns_error_not_panic() {
    let (dir, source) = file_source(b"abcdefghij");
    OpenOptions::new()
        .write(true)
        .open(dir.path().join("pack.bin"))
        .unwrap()
        .set_len(3)
        .unwrap();
    assert_eq!(source.len(), 10);
    assert_eq!(source.read_at(2, &mut [0; 5]), Err(PackError::Truncated));
    assert_eq!(source.read_at(5, &mut [0; 1]), Err(PackError::Truncated));
}

#[test]
fn concat_source_reads_across_chunk_boundary() {
    let (_dir, source) = concat(&[
        ("wiki.zimaa", b"abcd"),
        ("wiki.zimab", b"efgh"),
        ("wiki.zimac", b"ij"),
    ]);
    assert_eq!(source.len(), 10);
    let mut buf = [0; 7];
    assert_eq!(source.read_at(2, &mut buf).unwrap(), 7);
    assert_eq!(&buf, b"cdefghi");
}

#[test]
fn concat_sorts_parts_by_suffix_not_argument_order() {
    let (_dir, source) = concat(&[
        ("wiki.zimac", b"ij"),
        ("wiki.zimaa", b"abcd"),
        ("wiki.zimab", b"efgh"),
    ]);
    let mut buf = [0; 10];
    source.read_at(0, &mut buf).unwrap();
    assert_eq!(&buf, b"abcdefghij");
}

#[test]
fn concat_missing_middle_part_is_mismatch() {
    let (_dir, part) = file_source(b"abcd");
    let error = ConcatSource::from_parts(vec![
        ("wiki.zimaa".into(), part.clone()),
        ("wiki.zimac".into(), part),
    ]);
    assert!(matches!(error, Err(PackError::Mismatch(message)) if message.contains("ab")));
}

#[test]
fn concat_unequal_non_last_part_is_mismatch() {
    let (_dir, first) = file_source(b"abcd");
    let (_other, second) = file_source(b"efg");
    assert!(matches!(
        ConcatSource::from_parts(vec![
            ("wiki.zimaa".into(), first.clone()),
            ("wiki.zimab".into(), second),
            ("wiki.zimac".into(), first)
        ]),
        Err(PackError::Mismatch(_))
    ));
}

#[test]
fn concat_read_past_short_last_part_is_truncated() {
    let (_dir, source) = concat(&[("wiki.zimaa", b"abcd"), ("wiki.zimab", b"e")]);
    assert_eq!(source.read_at(3, &mut [0; 3]), Err(PackError::Truncated));
}

#[test]
fn concat_single_unsuffixed_file_is_accepted() {
    let (_dir, source) = concat(&[("wiki.zim", b"abc")]);
    let mut buf = [0; 3];
    source.read_at(0, &mut buf).unwrap();
    assert_eq!(&buf, b"abc");
}

#[test]
fn concat_poisoned_after_media_removed() {
    let (_dir, first) = file_source(b"abcd");
    let (_other, second) = file_source(b"efgh");
    let faulty = Arc::new(FaultySource::new(second, 1, PackError::MediaRemoved));
    let source = Arc::new(
        ConcatSource::from_parts(vec![
            ("wiki.zimaa".into(), first),
            ("wiki.zimab".into(), faulty),
        ])
        .unwrap(),
    );
    assert_eq!(source.read_at(3, &mut [0; 4]), Err(PackError::MediaRemoved));
    let other_reader = source.clone();
    assert_eq!(
        thread::spawn(move || other_reader.read_at(0, &mut [0; 1]))
            .join()
            .unwrap(),
        Err(PackError::MediaRemoved)
    );
    assert_eq!(source.read_at(0, &mut []), Err(PackError::MediaRemoved));
}

#[test]
fn concat_reopen_with_new_descriptors_after_reinsertion_reads_correctly() {
    let (dir, source) = file_source(b"abcdefgh");
    let old = ConcatSource::from_parts(vec![(
        "wiki.zim".into(),
        Arc::new(FaultySource::new(source, 0, PackError::MediaRemoved)),
    )])
    .unwrap();
    assert_eq!(old.read_at(0, &mut [0; 1]), Err(PackError::MediaRemoved));
    let reopened = ConcatSource::from_files(vec![(
        "wiki.zim".into(),
        File::open(dir.path().join("pack.bin")).unwrap(),
    )])
    .unwrap();
    let mut buf = [0; 8];
    reopened.read_at(0, &mut buf).unwrap();
    assert_eq!(&buf, b"abcdefgh");
    assert_eq!(old.read_at(0, &mut [0; 1]), Err(PackError::MediaRemoved));
}

#[test]
fn concat_rejects_empty_mixed_duplicate_and_wrong_group_parts() {
    assert!(matches!(
        ConcatSource::from_parts(vec![]),
        Err(PackError::Mismatch(_))
    ));
    let (_dir, part) = file_source(b"abcd");
    for names in [
        ["wiki.zimaa", "wiki.zimaa"],
        ["wiki.zimaa", "other.zimab"],
        ["wiki.zimaa", "wiki.zim"],
        ["wiki.zimab", "wiki.zimac"],
        ["wiki.zimAa", "wiki.zimab"],
    ] {
        assert!(
            matches!(
                ConcatSource::from_parts(vec![
                    (names[0].into(), part.clone()),
                    (names[1].into(), part.clone())
                ]),
                Err(PackError::Mismatch(_))
            ),
            "accepted {names:?}"
        );
    }
}

#[test]
fn sources_handle_empty_files_empty_buffers_and_eof_consistently() {
    let (dir, pread) = file_source(b"");
    let mmap = MmapSource::open(&dir.path().join("pack.bin")).unwrap();
    let window = WindowSource::new(pread.clone(), 0, 0).unwrap();
    let concat = ConcatSource::from_parts(vec![("empty.zim".into(), pread.clone())]).unwrap();
    for source in [&*pread, &mmap as &dyn PackSource, &window, &concat] {
        assert_eq!(source.len(), 0);
        assert_eq!(source.read_at(0, &mut []), Ok(0));
        assert_eq!(source.read_at(0, &mut [0]), Err(PackError::Truncated));
        assert_eq!(source.read_at(1, &mut []), Err(PackError::Truncated));
    }
}

struct SizedSource(u64);
impl PackSource for SizedSource {
    fn len(&self) -> u64 {
        self.0
    }
    fn read_at(&self, _off: u64, _buf: &mut [u8]) -> Result<usize, PackError> {
        panic!("bounds must reject before read")
    }
}

#[test]
fn source_ranges_and_concatenated_lengths_reject_overflow() {
    let huge: Arc<dyn PackSource> = Arc::new(SizedSource(u64::MAX));
    assert!(matches!(
        WindowSource::new(huge.clone(), u64::MAX, 1),
        Err(PackError::Mismatch(_))
    ));
    let window = WindowSource::new(huge.clone(), 0, u64::MAX).unwrap();
    assert_eq!(
        window.read_at(u64::MAX, &mut [0]),
        Err(PackError::Truncated)
    );
    assert!(matches!(
        ConcatSource::from_parts(vec![
            ("wiki.zimaa".into(), huge.clone()),
            ("wiki.zimab".into(), huge)
        ]),
        Err(PackError::Mismatch(_))
    ));
}

#[test]
fn positioned_reads_do_not_share_seek_state() {
    let (_dir, source) = file_source(b"0123456789");
    let workers: Vec<_> = (0..10)
        .map(|off| {
            let source = source.clone();
            thread::spawn(move || {
                for _ in 0..50 {
                    let mut buf = [0];
                    source.read_at(off, &mut buf).unwrap();
                    assert_eq!(buf[0], b'0' + off as u8);
                }
            })
        })
        .collect();
    for worker in workers {
        worker.join().unwrap();
    }
}
