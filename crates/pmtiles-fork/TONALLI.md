# Tonalli reader fork

Upstream: pmtiles 0.28.0, MIT OR Apache-2.0, revision
`dcd4051954792ff0d1a9390cc15a5381ca873a72`.
Official archive SHA-256:
`7a242b695fabc0870d663c3883e78ead4f888134c0e83fbf00fcc35d60005015`.
Original `LICENSE-MIT`, `LICENSE-APACHE`, README, manifest (`Cargo.toml.orig`),
VCS metadata and source structure are retained. No upstream branding is shown
in the Tonalli UI. This is the same reader, not a new implementation of v3.

Changed files:

- `Cargo.toml`: exact, reader-only dependency pins; no optional features exposed.
  Legacy cfg names remain recognized for the retained conditional source, but
  cannot be enabled through Cargo. No HTTP, S3, AWS, mmap, writer, Moka, iterator,
  TileJSON, Brotli or Zstd feature is available. Gzip/None only are supported.
- `src/error.rs`: typed truncation and budget errors.
- `src/header.rs`: reject non-v3 version byte (upstream checked only the magic);
  section-versus-source-length validation; fallible, capped Gzip
  decoding, checked None path; 16 MiB compressed, 64 MiB decoded, 1 MiB metadata.
  Gzip uses DEFLATE's fixed 32 KiB window. Unsupported codecs remain typed errors.
- `src/async_reader.rs`: cap exact reads before the backend; use the smaller
  decoded metadata limit; replace one file-backend doctest setup with an explicit
  memory AsyncBackend delegating positioned reads to the supported SyncBackend,
  so its example still compiles without the removed file feature.
  Remove the upstream `clippy::unused_async_trait_impl` lint name unsupported by
  the pinned Rust/Clippy toolchain; no broad unknown-lint suppression is used.
- `src/directory.rs`: reject empty directories; cap at 1,048,576 entries before
  fallible allocation. Existing checked varints, arithmetic and traversal depth
  remain. Core selects NoCache, not the unbounded HashMap cache.
- `src/header.rs`, `src/directory.rs`, `src/sync_reader.rs`: explicit ignore reasons
  on nine test functions that need external files omitted by the published crate.
  Two additional header magic/truncation tests use authored headers instead of
  loading that absent fixture; their original assertions remain active.

## Reproducible tests and external fixture limits

Normal workspace tests retain upstream header/coordinate/varint/corruption and
cache tests that require no external files. Nine fixture-dependent upstream test
functions are explicitly ignored rather than silently removed or reported as
passing. Their raster/vector/leaf fixtures are absent from the official crate
archive. If obtained from the exact upstream revision, with their own declared
licenses/notices retained, place them under this crate's `fixtures/` and run:

```
cargo test -p pmtiles --lib -- --ignored
```

The original optional asynchronous file/backend test module remains in source
under its original feature gate; those features are unavailable in this fork.
This does not claim that upstream's whole async/backend suite passed. Relevant
positioned-reader behavior instead has authored fixture coverage in
`../core/tests/tiles.rs` and `../core/tests/fixtures/pmtiles/`: valid raw/Gzip root
and leaf lookup, missing tiles, bounded cycles/depth, varints/overflows, budgets,
metadata, short reads, EOF, source errors/removal and concurrent reads.

The three real city extracts are deliberately external to a fresh checkout.
Bootstrap their exact manifest entries, then run the explicitly ignored test:

```
spike-cli fetch --id map-mx-mexico-city-z8
spike-cli fetch --id map-us-san-diego-z8
spike-cli fetch --id map-ca-montreal-z8
cargo test -p tonalli-core --test tiles actual_three_city_extracts_decode -- --ignored
```

Neither this notice nor the existence of tests claims GREEN; actual command
results belong in the spike validation evidence after execution.
