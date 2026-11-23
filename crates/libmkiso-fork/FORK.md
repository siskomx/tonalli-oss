# Tonalli libmkiso reader fork

Upstream: libmkiso0.1.5, MIT, published2026-10-13.
Archive: https://static.crates.io/crates/libmkiso/libmkiso-0.1.5.crate
Bytes:247321. SHA-256:
`262387b6106a83fc5a389eba8a513fee4024ef83e60f22aff8a3bbe462b03289`.
Upstream Git revision: `e5d8e75bbf9dd5dd33c0c6b12a9a0686efe916e9`.
All published files were copied verbatim and verified before local additions;
upstream LICENSE and copyright are retained. Published Cargo.toml is unchanged.

Tonalli uses the read-only API with default features disabled. No writer/CLI
feature is enabled by core. Local changes are deliberately narrow:

- This provenance file.
- `tests/udf_profiles.rs`: small mixed-map sparing-address regression, authored
  before the correction. Actual debug RED must be recorded before fixing it.
- `tests/iso9660_extended.rs`: scope the Unix-only metadata import to its
  Unix-only test so the upstream all-target lint command also passes on Windows.
- `src/udf_partition.rs`: widen the replacement LBA and in-packet displacement
  before addition, preserving virtual/sparable/metadata map support. Both are
  bounded integer fields; their widened sum and multiplication by 2048 fit u64.
  Existing source-range and descriptor checks still validate the mapped bytes.

The 667,648-byte mixed-map regression actually panicked at the old arithmetic
in a debug build before this fix (`target/libmkiso-sparing-red.log`). The public
Tonalli test separately failed on its missing ISO API/dependency
(`target/iso-core-red.log`); both REDs preceded implementation. The input-size
cap did not prevent the bug. Post-fix checks remain pending below.

The original registry contents and per-file hashes are recorded locally in
`target/libmkiso-upstream-verbatim.json`. Platform builds and interoperability
remain separate validation work; publishing the fork is not evidence they pass.
