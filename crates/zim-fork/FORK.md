# Tonalli fork of zim 0.5.0

Upstream: https://github.com/dignifiedquire/zim, commit
`82b897e479254601efdabc0f37c1cb9e7e20f7d3`. The official crate archive SHA-256
is `63cffa116b0aaebeb94142acd0ad39c63ee9f6180a5729e5c02036b73b4b082e`.
The original Apache-2.0 and MIT texts are retained unchanged. Tonalli changes
are also available under either license. This is a local fork, not an
unmodified registry release.

Changed files: Cargo.toml, src/lib.rs, src/errors.rs, src/store.rs,
src/cluster.rs, src/zim.rs and src/directory_entry.rs. src/tests.rs is new.
The original CLI programs and their clap/indicatif/pbr/hex/num-format/rayon/
stopwatch dependencies are excluded. Library compression now shares the
workspace's zstd 0.14.3 and static xz2 0.1.7.

Store accepts a small ReadAt adapter instead of opening/mapping paths. Core
owns the PackSource implementation, so this crate never selects mmap for
removable media. Source error causes survive the adapter for typed recovery.
Directory records are read in bounded 128-byte increments (64 KiB maximum).
The existing streaming MD5 engine polls a caller-owned control callback before
reading and after each 64 KiB checksum chunk. Interruption returns its payload
without claiming checksum verification; the original verifier always continues.
Listings retain a resolved location, read lengths/individual indices by range,
and reject counts exceeding the directory count. Legacy lists sort by
namespace and title; callers must preserve that ordering in binary searches.

Uncompressed blobs use positioned ranges, including their adjacent offset
entries. Compressed input is streamed with a 64 MiB XZ decoder memory limit
and zstd window log cap 26. A per-archive decoder permit bounds concurrent
decoder working sets. The decoded-cluster LRU holds at most 64 MiB; larger
clusters stream requested windows without entering the cache. These are M1
spike settings, not measured phone budgets. Whole-body calls explicitly copy
their requested blob; core bounds its article response to 64 MiB, and larger
resources require a future ranged serving API.

Validation and remaining performance/device gates:
[zim-validation.md](../../docs/spike/zim-validation.md).
