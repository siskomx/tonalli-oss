# Country extraction adapter

Pinned go-pmtiles v1.31.2 algorithms, also used by NOMAD's country extractor.
The copied algorithm has no HTTP/cloud/server backend: Rust supplies validated
range-file handles through the platform-owned Wi-Fi client. Rust also owns job
admission, attempt fencing, cache hashes, disk budgets and staged activation.

Build with installed Go 1.25.2 and the existing Tonalli native toolchains:

```powershell
rtk proxy powershell -NoProfile -File native/maps-extract/build.ps1 Windows
rtk proxy powershell -NoProfile -File android/build-core.ps1
```

The Windows bundle must include the resulting `tonalli_maps.dll` beside the EXE.
The Android script builds, verifies the pinned Go CPU overlay and packages the
ARM64 library. The overlay skips optional MIDR/Neoverse tuning on Android because
API34 NDK translation advertises CPUID but traps the register read. It preserves
ordinary HWCAP detection and never edits the installed Go toolchain.

Algorithm checks: `rtk proxy go test ./pmtiles` in this directory. Core country
integration tests cover range snapshot/prefix verification, admission, import
and stale native stops. Full Core unit tests currently have unrelated unfinished
index-test compile errors; do not edit that parked work to run these checks.

To reproduce the three geographic regions, fetch the exact NOMAD source pinned
in `countries/UPSTREAM.json` and run its full Natural Earth file through:

```powershell
rtk proxy node --experimental-strip-types native/maps-extract/countries/generate.mjs PATH_TO_PINNED_GEODATA
```

The generator checks the original dataset hash and uses NOMAD's actual buffering
function. Natural Earth data is public domain; algorithm/runtime/NOMAD source
licenses and hashes are recorded separately. Full license texts are in `licenses`.

The extractor caps directories at 8 MiB and aggregate entries at 500,000.
Overview countries and standard Mexico are checked; detailed large countries may
exceed that budget and return a preparation error. This does not close phone RAM,
GPS accuracy, performance or full-v1 acceptance gates.
