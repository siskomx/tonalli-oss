# Authored PMTiles test data

Generated for Tonalli, with no upstream fixture dependency. `mod.rs` constructs
v3 headers/directories and tiny gzip stored blocks in memory. The tile payload
is a minimal MVT with one named layer, version 2 and extent 4096, no features.

- `decoded-over-64m.gz`: gzip of 64 MiB + 1 ASCII X bytes, generated incrementally
  with Python zlib level 9 / wbits 31. Tests the decoded-output budget.
- `decoded-at-64m.gz`: the corresponding exactly-64-MiB acceptance boundary.
- `entries-over-limit.gz`: gzip of a four-column directory declaring 1,048,577
  entries: first TileID 0 then deltas 1, run lengths 1, lengths 1, first encoded
  offset 1 then contiguous offsets 0. Tests the entry budget before allocation.

These compact adversarial inputs are test fixtures, not installed map packs.
Both compressed bytes and expanded sizes are intentionally checked by the tests.
Actual city extract checks are explicitly ignored on clean checkouts; their test
message names the three `spike-cli fetch --id` bootstrap commands and the explicit
`cargo test` invocation. They must be run during Task 6 validation when installed.
