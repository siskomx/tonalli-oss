use crate::{Cluster, ReadAt, Store};
use std::io::{self, Write};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
struct Bytes {
    bytes: Vec<u8>,
    reads: AtomicUsize,
}
impl ReadAt for Bytes {
    fn len(&self) -> u64 {
        self.bytes.len() as u64
    }
    fn read_at(&self, off: u64, out: &mut [u8]) -> io::Result<usize> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        let start = off as usize;
        let count = out.len().min(self.bytes.len().saturating_sub(start));
        out[..count].copy_from_slice(&self.bytes[start..start + count]);
        Ok(count)
    }
}
fn payload() -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&8_u32.to_le_bytes());
    out.extend_from_slice(&13_u32.to_le_bytes());
    out.extend_from_slice(b"hello");
    out
}
#[test]
fn xz_and_zstd_decode_and_reuse_cached_cluster() {
    let mut xz = xz2::write::XzEncoder::new(Vec::new(), 6);
    xz.write_all(&payload()).unwrap();
    for (info, data) in [
        (4, xz.finish().unwrap()),
        (5, zstd::stream::encode_all(&payload()[..], 1).unwrap()),
    ] {
        let mut bytes = vec![info];
        bytes.extend(data);
        let source = Arc::new(Bytes {
            bytes,
            reads: AtomicUsize::new(0),
        });
        let store = Store::from_source(source.clone());
        let cluster = Cluster::new(&store, 0, store.len(), 6).unwrap();
        assert_eq!(&*cluster.read().unwrap().blob(0).unwrap(), b"hello");
        let reads = source.reads.load(Ordering::SeqCst);
        assert_eq!(
            &*cluster.read().unwrap().blob_range(0, 1, 3).unwrap(),
            b"ell"
        );
        assert_eq!(source.reads.load(Ordering::SeqCst), reads);
    }
}
struct Large;
const LARGE: u64 = 8 * 1024 * 1024 * 1024;
impl ReadAt for Large {
    fn len(&self) -> u64 {
        LARGE + 1
    }
    fn read_at(&self, offset: u64, out: &mut [u8]) -> io::Result<usize> {
        assert!(out.len() <= 4096, "whole cluster read attempted");
        for (i, b) in out.iter_mut().enumerate() {
            let at = offset + i as u64;
            *b = match at {
                0 => 0x11,
                1..=8 => 16_u64.to_le_bytes()[(at - 1) as usize],
                9..=16 => LARGE.to_le_bytes()[(at - 9) as usize],
                _ => 42,
            };
        }
        Ok(out.len())
    }
}
#[test]
fn eight_gib_uncompressed_blob_uses_only_requested_ranges() {
    let store = Store::from_source(Arc::new(Large));
    let cluster = Cluster::new(&store, 0, store.len(), 6).unwrap();
    let guard = cluster.read().unwrap();
    assert_eq!(guard.blob_len(0).unwrap(), LARGE - 16);
    assert_eq!(&*guard.blob_range(0, 123456, 4).unwrap(), [42; 4]);
}
#[test]
fn malformed_offsets_and_large_decoder_window_return_errors() {
    for bytes in [
        vec![1, 0, 0, 0, 0],
        vec![
            5, 0x28, 0xb5, 0x2f, 0xfd, 0, 0x88, 0x69, 0, 0, 8, 0, 0, 0, 13, 0, 0, 0, b'h', b'e',
            b'l', b'l', b'o',
        ],
    ] {
        let source = Arc::new(Bytes {
            bytes,
            reads: AtomicUsize::new(0),
        });
        let store = Store::from_source(source);
        let cluster = Cluster::new(&store, 0, store.len(), 6).unwrap();
        assert!(cluster.read().and_then(|g| g.blob_len(0)).is_err());
    }
}

#[test]
fn oversized_xz_dictionary_is_rejected_before_output_allocation() {
    // Valid XZ block header with a 4 GiB LZMA2 dictionary and corrected header CRC.
    let hex="fd377a585a000004e6d6b4460200210128000000e6a011b301000c080000000d00000068656c6c6f000000002fd93d46cd3f87170001250d7119c4b61fb6f37d010000000004595a";
    let mut bytes = vec![4];
    for i in (0..hex.len()).step_by(2) {
        bytes.push(u8::from_str_radix(&hex[i..i + 2], 16).unwrap());
    }
    let source = Arc::new(Bytes {
        bytes,
        reads: AtomicUsize::new(0),
    });
    let store = Store::from_source(source);
    let cluster = Cluster::new(&store, 0, store.len(), 6).unwrap();
    assert!(cluster.read().is_err());
}

#[test]
fn concurrent_reads_decode_a_cached_cluster_once() {
    let mut bytes = vec![5];
    bytes.extend(zstd::stream::encode_all(&payload()[..], 1).unwrap());
    let source = Arc::new(Bytes {
        bytes,
        reads: AtomicUsize::new(0),
    });
    let store = Store::from_source(source.clone());
    let cluster = Cluster::new(&store, 0, store.len(), 6).unwrap();
    std::thread::scope(|scope| {
        for _ in 0..8 {
            let cluster = &cluster;
            scope.spawn(move || assert_eq!(&*cluster.read().unwrap().blob(0).unwrap(), b"hello"));
        }
    });
    assert_eq!(source.reads.load(Ordering::SeqCst), 2);
}

#[test]
fn oversized_compressed_cluster_streams_a_small_requested_range() {
    let length = crate::store::CACHE_BYTES + 1;
    let mut encoder = zstd::stream::write::Encoder::new(Vec::new(), 1).unwrap();
    encoder.write_all(&8_u32.to_le_bytes()).unwrap();
    encoder
        .write_all(&((length + 8) as u32).to_le_bytes())
        .unwrap();
    let chunk = [7; 64 * 1024];
    for _ in 0..length / chunk.len() {
        encoder.write_all(&chunk).unwrap();
    }
    encoder.write_all(&chunk[..length % chunk.len()]).unwrap();
    let mut bytes = vec![5];
    bytes.extend(encoder.finish().unwrap());
    let store = Store::from_source(Arc::new(Bytes {
        bytes,
        reads: AtomicUsize::new(0),
    }));
    let cluster = Cluster::new(&store, 0, store.len(), 6).unwrap();
    assert_eq!(
        &*cluster.read().unwrap().blob_range(0, 123, 4).unwrap(),
        [7; 4]
    );
    assert!(!cluster.is_loaded());
}
