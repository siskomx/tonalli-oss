# Android M1 probe

This thin Kotlin test app implements Task 8 Step 2's title-suggestion probe
and Step 2b's small-index copy/search/mismatch probe. **Open shared offline UI**
hosts the shared React UI in a separate WebView backed by the Rust loopback
server. Emulator functional evidence is recorded in `docs/spike/RESULTS.md`;
physical-phone acceptance remains open.

## Build on the prepared Windows host

The `lan/` folder implements the M1 **LAN sharing** probe, a non-exported
foreground service and numeric Wi-Fi bind policies. Opening its controls leaves
sharing off; Start explicitly requests permissions and starts a Stop notification.
It shares only the installed small `probe.zim` and `map-{mx,us,ca}.pmtiles` inputs
in app-specific external storage. The private bootstrap is closed after registry
transfer; notes and settings are never registered. Wi-Fi loss, system low battery
and explicit Stop release its resources. Local-only hotspot and optional
30-minute wake lock are explicit controls; actual network/OEM checks remain open.
Run its JVM checks without building native/UI inputs:

```powershell
rtk proxy powershell -NoProfile -ExecutionPolicy Bypass -File android/gradle-local.ps1 :app:checkLan --no-daemon --max-workers=1
```

From the repository root in PowerShell:

```powershell
rtk proxy powershell -NoProfile -ExecutionPolicy Bypass -File android/gradle-local.ps1 assembleDebug --no-daemon
rtk proxy powershell -NoProfile -ExecutionPolicy Bypass -File android/verify-apk.ps1
```

The Gradle `buildCore` task invokes cargo-ndk for release `arm64-v8a`, API 34,
then generates matching UniFFI Kotlin source from that library. It uses the
existing local toolchain helper and the `target/android-native` Cargo cache.
For a separate bounded native rebuild, run
`rtk proxy powershell -NoProfile -ExecutionPolicy Bypass -File android/build-core.ps1 -Jobs 1`.
The job limit applies to both ARM64 compilation and host binding generation.
Optional AI native builds require `-Ai` on that script or `-PenableAi=true` on Gradle.
AI builds use `ffi,ai`, force one Cargo/CMake job, and select the prepared Android
Ninja generator; its process environment is restored before host binding generation.
The default build remains `ffi` only and selects the no-AI probe stub. AI JNI/Kotlin
outputs are isolated under `app/build/generated/ai`; normal outputs stay directly
under `app/build/generated`. Gradle selects the matching profile even when skipping
the native build; pass `-Ai` to `verify-apk.ps1` for an AI APK.
`-DryRun` prints the actual planned arguments, output paths and CMake settings without starting tools or changing artifacts; run
`rtk proxy powershell -NoProfile -ExecutionPolicy Bypass -File android/test-build-core.ps1`
for the focused profile/environment checks. Phone import/load, short generation
and one-text embedding bridge checks are recorded in RESULTS; sustained AI,
twenty-document re-ranking and tier-specific acceptance remain separate gates.
`AiModelSpec` and `AiModelFiles` prepare one verified internal GGUF without loading
it or changing activation. Their host-only task is `:app:checkAiModelFiles` through
`gradle-local.ps1` with `-PskipCoreBuild -PskipWebBuild`; use the bounded JVM-only
profile recorded in RESULTS. The private AI M1 probe uses exact catalog pins, a
request-bound app-owned incoming folder, explicit import/load and bounded generation.
Its debug preparation action is `org.tonalli.spike.PREPARE_AI_INPUT` on MainActivity,
with canonical UUID `request_id` and catalog `model_id`; the private receipt is
`files/ai-inputs-ready/<request_id>.json`. Push only `model.gguf` into the receipt's
incoming folder, then enter that request UUID in the probe. Import does not depend
on RAM tier. Core checks actual runtime RAM before execution; Lite refusal is expected.
The embedding action runs `aiEmbed` on the worker, validates owned finite/unit
vectors and closes the model. Its actual native token limit remains enforced by
Core; the API does not return token counts. Reload before another request.

Explicit comparison fixtures use integer `output_tokens` (32 or128) and paired
nonblank `question`/`context` intent extras. Normal generation defaults to32 and
records no prompt history. In a debuggable build, launch Main with those three
extras, then tap its existing AI button; it forwards only those keys to the
private probe. The probe remains non-exported, so direct shell launches are
denied. Start a fresh owned idle app instance for each fixture because the
private activity is singleTask; never stop an active measurement to change it.
The composed fixture prompt must remain unchanged and fit64KiB UTF-8.

Only paired fixtures write bounded raw output/timestamps to a new private
`files/ai-measurements/<UUID>/fixture.json`. Initial metadata is durable before
submission; only a real Core terminal followed by model cleanup publishes a
complete trace. Omissions, cancellation and failures remain ineligible. Load,
retrieval and staging are excluded from its request timings; useful-answer time
stays unavailable until raw-output inspection. Trace completeness is not a
performance/quality gate. Screen-on applies only while a fixture/measurement is
active. The latest phone trial and Core baseline differ after the unsuccessful
batch128 experiment; consult RESULTS and the artifact receipt before comparing.
The local driver selects the installed JDK 17 and SDK for its process only.
`-PskipCoreBuild` is only for checking already generated matching artifacts.
The default `buildWeb` task runs the shared UI build and checksum packaging;
`-PskipWebBuild` is only for already generated UI assets. The mandatory artifact
check still verifies every file when that flag is used.

The installable development APK is `android/app/build/outputs/apk/debug/app-debug.apk`.
It uses Gradle's local debug signing key and is not a distributable signed release.
The release APK is unsigned until a user-owned release key is configured.
Only ARM64 is packaged; both variants use the Rust release library.

## Real phone check (pending)

Connect and authorize an ARM64 Android phone. API 34 is an explicit temporary
minimum pending the user's reported version. First inspect `adb devices -l`;
when multiple devices are present, explicitly select the intended device with
`-s SERIAL` for every command. `adb` is selected from the local SDK without
requiring a global PATH change.

```powershell
$adb = Join-Path $env:LOCALAPPDATA 'Android/Sdk/platform-tools/adb.exe'
rtk proxy $adb devices -l
$serial = 'REPLACE_WITH_AUTHORIZED_DEVICE_SERIAL'
rtk proxy $adb -s $serial install -r android/app/build/outputs/apk/debug/app-debug.apk
rtk proxy $adb -s $serial shell am start -n org.tonalli.spike/.MainActivity
rtk proxy $adb -s $serial push docs/spike/packs/wikipedia_en_knots_maxi_2026-10.zim /sdcard/Android/data/org.tonalli.spike/files/probe.zim
```

Use the actual verified small English fixture path from `docs/spike/packs.toml`
if the example filename differs. Launch once before pushing so Android creates
the app-specific external folder. Tap **Run ZIM title probe** and record the
displayed suggestions and runtime RAM/Android values. The default prefix is
`Trefoil` (the fixture contains `Trefoil knot`); pass `--es prefix WORD` to
`am start` for another known fixture title prefix.
Capture the local development diagnostics for the gate record:

```powershell
rtk proxy $adb -s $serial logcat -d -s TonalliM1:I '*:S'
```

Reading runs off the UI thread. Kotlin detaches one read-only descriptor and
Rust owns/closes it on every outcome; Kotlin closes only the detached wrapper.

Pushes go to app-specific external storage without broad storage permission.
Indexes and encrypted notes must be placed in app-internal storage. Do not
interpret an APK build as device, portability, WebView,
notes, memory, SD-removal or performance evidence.

## Small Windows index check (device result pending)

The prepared immutable transfer bundle is
`target/android-index-probe/v2/probe-index/`. Its `snapshot.json` must exactly
match the trusted manifest packaged in the APK. To reproduce the export to a
new destination, use the prepared host's Python **3.12 or newer**:

```powershell
rtk proxy python scripts/export-index-probe.py target/index-v2-small-20261111-r1 target/android-index-probe/repeat/probe-index
```

The exporter refuses existing destinations, copies `CURRENT` and its completed
generation, and omits the two Tantivy runtime lock files. It never builds an
index. Do not refresh the trusted APK manifest merely to bypass a checksum
failure; a changed Windows fixture needs a separately checked snapshot.
The current manifest SHA-256 is
`c35c13df8ed4ed34fa77a23e8e5d69f016c93742b4649c871cf294855709e4a0`
(1,859 bytes; ten payload files totaling 1,496,177 bytes). Body metadata format
2 records the Unicode analyzer identity; old v1 bundles remain historical.

After installation and one initial launch, push the checked English ZIM as
`probe.zim` as above, the deliberately different Spanish ZIM and the bundle:

```powershell
rtk proxy $adb -s $serial push docs/spike/packs/wikipedia_es_climate-change_maxi_2026-10.zim /sdcard/Android/data/org.tonalli.spike/files/wrong-probe.zim
rtk proxy python scripts/push_index_probe.py --serial $serial --adb $adb --bundle target/android-index-probe/v2/probe-index --trusted-manifest android/app/src/main/assets/index-probe-manifest.json
```

Tap **Run small index portability probe**. The worker copies into app-internal
storage, verifies every payload length/SHA-256, opens it through the native FFI,
checks all three ordered Windows top-five title lists and specifically requires
`PackException.Mismatch` for the wrong ZIM. Native index handles are destroyed
before atomic publication of the immutable internal snapshot. Failure preserves
previous snapshots. Later runs reverify/reuse the installed snapshot.
The incoming folder is derived from the trusted manifest SHA prefix. The helper
asks the debug app to create the exact trusted transfer directories as the app
UID, then pushes individual files (snapshot last). Recursive `adb push` creates
shell-owned directories that this Android external-storage mount can deny to
the app; even `run-as mkdir` may use a different mount namespace. Wrong-owned
existing directories are preserved/refused, with no chmod/deletion fallback.
Do not merge another generation into an existing incoming directory.

Record the on-screen results and `TonalliM1` logcat output, including URLs and
the internal path. Physical-device Step 2b remains open until the user performs
this check. Do not build the large index before its success is reviewed.

The pure JVM staging self-check uses the existing Kotlin compiler, without
JUnit or new dependencies:

```powershell
rtk proxy powershell -NoProfile -ExecutionPolicy Bypass -File android/gradle-local.ps1 checkIndexSnapshot --no-daemon
```

## Shared WebView UI (emulator checked; phone evidence separate)

Tap **Open shared offline UI**. The dedicated private, single-task screen stages
the packaged UI into immutable internal `files/web-ui/ui-<id>/` directories,
verifies every checksum and starts Rust on `127.0.0.1` with a random port.
It loads the fresh one-use native launch URL with ordinary WebView cookies.
Only the known `probe.zim` is registered as a ZIM; the index, wrong-ZIM fixture,
notes and arbitrary external files are never automatically registered.

Optional checked map fixtures can be pushed before opening that screen:

```powershell
rtk proxy $adb -s $serial push docs/spike/packs/mx-mexico-city-z8.pmtiles /sdcard/Android/data/org.tonalli.spike/files/map-mx.pmtiles
rtk proxy $adb -s $serial push docs/spike/packs/us-san-diego-z8.pmtiles /sdcard/Android/data/org.tonalli.spike/files/map-us.pmtiles
rtk proxy $adb -s $serial push docs/spike/packs/ca-montreal-z8.pmtiles /sdcard/Android/data/org.tonalli.spike/files/map-ca.pmtiles
```

Each present map goes through native format/metadata validation. Missing external
storage or fixtures still permits the internal UI to start; an empty map screen
does not pass the map-rendering gate. Close and reopen the shared UI screen after
changing fixtures. **Close shared UI**, destruction or a renderer failure shuts
down its native server on the worker. Rapid repeated launches reuse one screen.

HTTP requests must use that exact loopback address/port; top-level navigation
is limited to app routes. File/content access, popups, downloads and WebView
permissions are disabled. No JavaScript bridge exists. The packaged CSP and
article iframe sandbox remain unchanged. Inline data resources and local blob
map workers remain supported; unused service-worker networking is disabled for
this M1 stub. Normal cache policy is unchanged. Online reputation lookup is
disabled because this screen only browses the app-owned loopback server.

The only Android permission is `INTERNET`; cleartext is denied by default with
an exact `127.0.0.1` exception. This is a WebView request policy, not a general
OS firewall. Debuggable APKs expose WebView inspection through authorized ADB;
release APKs disable that debugging. Native code never fabricates a response
status/readiness result or relaxes the shared article checks.

Record actual WebView provider, article 2xx/MIME readiness, subsequent 401/503
document failures, map worker cookie behavior, rendered map evidence and screen
close/reopen/rapid-launch cleanup. Unknown `responseStatus` remains fail closed;
a built APK does not establish Android article support. The fixed private core
data directory is `files/core/`, separate from served UI versions; future notes
use `files/core/notes.db` and `notes.keys` and are never static assets.

```powershell
rtk proxy powershell -NoProfile -ExecutionPolicy Bypass -File android/gradle-local.ps1 checkWebShell --no-daemon
```

## Native encrypted-notes check (emulator checked; phone evidence separate)

Close the shared UI screen before this test, then tap **Open encrypted notes
probe**. This disposable native screen uses only `files/core/notes.db` and
`files/core/notes.keys`. It never resets existing files. Empty credentials are
rejected; nonempty input, including leading/trailing spaces, is passed verbatim.

1. Enter a private test passphrase and tap **Create new vault only**. Both files
   must be absent. Privately record the recovery key immediately; it is shown
   once, cleared by any action/pause, and never saved by the app. Do not send the
   key, passwords, screenshots or UI dumps containing them.
2. Re-enter the passphrase, **Open with passphrase**, then **Add and verify
   synthetic sentinel** and **Close vault**. Repeated Add does not duplicate the
   same sentinel. No note title/body is printed in logs.
3. Force-stop and restart using the selected device, reopen this screen, unlock
   with the passphrase and **Read synthetic sentinel**:

   ```powershell
   rtk proxy $adb -s $serial shell am force-stop org.tonalli.spike
   rtk proxy $adb -s $serial shell am start -n org.tonalli.spike/.MainActivity
   ```

4. Enter a deliberately different passphrase and tap **Check wrong passphrase
   (closes first)**. Only typed `UnlockFailed` plus unchanged DB/key SHA-256 and
   sizes passes. An accidentally valid credential is explicitly a failed test;
   it is closed again and nothing is reset.
5. Enter the separately recorded key in the recovery field, **Open with
   recovery**, then **Read synthetic sentinel** and **Close vault**.
6. Force-stop again. Export/check the ciphertext using the binary-safe helper
   and a new output directory (replace the serial with the intended authorized
   device; no credentials belong in a command):

   ```powershell
   rtk proxy $adb -s $serial shell am force-stop org.tonalli.spike
   rtk proxy python android/export-notes-probe.py --serial $serial target/notes-device-export/attempt-1
   rtk proxy $adb -s $serial logcat -d -s TonalliM1:I '*:S'
   ```

The export helper reads fixed app-owned files with `adb exec-out run-as`, writes
through Python binary file handles, compares device SHA-256 before/after export
and verifies matching local bytes, absent synthetic marker and absent plaintext
SQLite header. **Do not redirect binary DB/key stdout with PowerShell 5 `>`**;
it can transcode the files. The helper refuses an existing destination and
retains any failed/partial export for review. It prints only fingerprints and
check results, never contents. Share those results and sanitized logs, not the
raw vault, passphrase or recovery key.

The screen uses `FLAG_SECURE`, masked unsaved fields and no autofill/content
capture. Pause clears secrets and queues vault close on the worker; destruction
also rejects late native handles. Queued actions are cancelled on pause. An
already-running Create may still complete; its encrypted files are preserved,
its late key display is suppressed, and the passphrase remains necessary to
open it. There is no automatic reset or second recovery-key display. JVM/FFI
immutable strings cannot promise immediate memory wiping.

```powershell
rtk proxy powershell -NoProfile -ExecutionPolicy Bypass -File android/gradle-local.ps1 checkNotesProbe --no-daemon
rtk proxy python android/test-notes-export.py
```

## Shared notes UI (emulator checked; phone evidence separate)

Initialize a new vault once with the native notes probe above, privately record
its recovery key, then **Close vault** and leave that screen. Open **shared
offline UI** and select **Notes** (or **Notas** in Spanish). The shared route is
`/notes`; it uses the same fixed encrypted vault, not a separate browser vault.
There is no HTTP create/reset control.

Use the shared controls to unlock with passphrase or recovery key, save a
synthetic test note, list/select it and run literal text search. **Lock and clear
notes** clears displayed/draft content and requires a confirmed server lock.
If lock is unconfirmed, use the retry control; cleared UI alone is not proof the
server locked. Leaving Notes clears its component state and attempts a separate
best-effort lock; close the entire shared UI screen to shut down the native
server. A fresh server starts locked.

## Native benchmark preparation (emulator checked; phone evidence separate)

Open **Open internal benchmark probe** and select a trusted named dataset. Use
**Prepare selected dataset** only after copying its matching host export to
app-specific external `benchmark/<id>/` (`manifest.json`, `pack.zim`,
`titles.bin`). Preparation verifies the packaged manifest and every payload
SHA-256, validates the native title table, then atomically installs an immutable
private copy. It does not delete older installations. Full input requires over
53 GB of additional internal space; the small knots input validates tooling
only and cannot satisfy representative performance acceptance.

The warm control records one untimed prime and 30 timed calls for each operation.
The explicit-serial host driver in `scripts/phone_benchmark.py` uses separate
fresh processes for single suggest/article samples and reads atomic private
JSON results. It never automatically copies large input. Use its `--help` and
the checked input receipts before running; retain raw samples and device
metadata. Title attachment checksum scanning warms caches, so fresh process
does not mean OS-cold. No cache eviction or performance verdict is implied.

The debug-only WebView measurement actions use the same already-prepared
internal data, a separate retained profile and a deferred blank-page launch.
The driver must verify its safe native session receipt, installed APK and UI
identity before observing a page. Normal shared UI still uses the existing
optional external fixtures. See `docs/spike/android-benchmark-validation.md`
for contracts, limitations and current automated evidence. Actual phone timing,
memory and rendered WebView checks remain pending.

For **SAF positioned-read benchmarks**, select/reselect the standalone ZIM or
split group in the removable probe first. Each successful selection save gets
a new random revision, including an identical re-selection; old manual receipts
then become invalid. The safe private `files/benchmarks/saf-selection.json`
summary contains only revision, fingerprint, kind and part count, never URIs or
display names. Earlier selection records still reopen functionally but need one
re-selection before benchmarking. ISO selections are excluded from this series.

Prepare the benchmark transfer directories with the debug
`PREPARE_PROBE_INPUTS` protocol, then push the selected dataset's exact
`manifest.json` and `titles.bin` individually to `benchmark/<id>/`. Use
**Prepare SAF benchmark title table only**: it verifies/stages only the table
into private `benchmarks/tables/<manifest-SHA>/` and validates attachment to the
selected pack. The existing two-file internal benchmark preparation is unchanged.

Provide `benchmark/saf-receipt.json` with the exact manual receipt schema agreed
with `scripts/phone_benchmark.py`: trusted dataset ID/manifest SHA, current
selection revision/fingerprint, externally observed logical-ZIM SHA/bytes and
UTC time, medium, card class, filesystem, and
`evidence: "external_manual_not_rechecked"`. Bind it explicitly with the native
button or debug `SAF_BENCHMARK_BIND` action. This records an external statement;
the app does not rehash current SAF source bytes. It checks observable part
lengths when available and rejects changed metadata or selection revisions.

The warm SAF button and debug `SAF_BENCHMARK` action reuse the same timing
series. The explicit-serial collector accepts `collect --saf-receipt PATH` and
performs the same 61 launches as the internal collector. Results label
`source: "saf_pread"`, expected dataset hash/bytes and external manual evidence
separately. A provider-backed source is not proof of physical SD media; emulator
tooling results never satisfy phone latency or real-card hardware gates.

## Trusted v2 index handoff (coding preparation)

The separate **Validate bound SAF index handoff (selected dataset)** control is
v2 coding preparation. Its small fixture has a trusted packaged
`index-probe-small.json`; a full descriptor remains unavailable while the full
build retry runs. Full artifact consumption/activation remains held. The
old small-index control and its 32 MiB profile are unchanged.

The v2 path uses an explicitly bound SAF ZIM/split selection and manual receipt,
without requiring an internal ZIM copy or prepared title table. Its exact
packaged descriptor defines checksums, file lengths and the three ordered
title/URL baselines. Metadata is bounded to 1 MiB/4,096 files with checked Long
totals. The app prepares owned incoming directories only after budgeting incoming
plus internal staging plus 32 MiB reserve. After delivery, only the remaining
internal staging space is required; incoming bytes are already on disk.

For the independently checked small export, the new host handoff command is:

```powershell
rtk proxy python scripts/push_index_handoff.py --export-receipt target/index-handoff-small-v2/export-receipt.json --trusted-manifest android/app/src/main/assets/index-probe-small.json --serial $serial --adb $adb --evidence target/index-handoff-delivery-small-r1
```

This is delivery, not an installed-index result. The worker streams positioned
reads into private staging, verifies all payloads, opens fresh bound SAF
descriptors, checks every ordered title and URL, closes index/pack handles and
activates atomically under the selection fence. Cancellation, failed parity,
corruption or observed media removal preserves older snapshots. This path still
has passed its generic small-fixture replay on API34; it supplies no phone
performance or current source-byte SHA claim.

The same validation tap primes each trusted query once, then records 30 warm
native `indexSearch` calls per query. Ordered title/URL checks and selection/
cancellation fences run outside each timer. Copy, checksum and source/index-open
setup are excluded and OS caches are untouched. A private
`files/index-handoff-results/<UUID>.json` records raw arrays, nearest-rank p50/p95,
setup times and the bound manual-source identity. Failed/cancelled runs retain
partial arrays as failed reports without success percentiles. Success is
published only after the final source fence and installation finish. Dataset
eligibility alone does not close a phone performance gate.

## Removable documents and ISO probe (emulator checked; phone evidence separate)

Open **Open removable document probe**. Use the system file picker for a single
ZIM or all split parts, or select a folder and then one immediate pack group.
The probe saves only the returned persistable read access and opaque URI/name
selection privately. It does not copy, install or serve the chosen pack. Tap
**Reopen saved selection with fresh descriptors**, then search a known title
prefix and read a known article URL. Display names and ISO paths are preserved
verbatim; the core sorts and validates split filenames.

For the authored `crates/core/tests/fixtures/iso/plain-v1.iso`, enter
`/FIXTURE.ZIM` before selecting the ISO. After Reopen, prefix `English` should
find the benign fixture article; use a returned URL in the explicit article
field. Use the checked ISO manifest for other exact inner paths and hashes.
ISO content is read in place through one descriptor, without extraction/mount.

Record actual medium, card class/filesystem and known input hashes separately:
SAF access proves provider-backed positioned reads, not physical SD identity.
After process restart or reinsertion explicitly reopen; a revoked read grant
requires selecting again. If the core reports **Media removed**, queued reads
are cancelled and old handles are closed. Android may instead terminate the
process during unmount; record which occurred and verify clean relaunch/private
data. Do not present host tests as those device results. Small-index parity is
functionally checked on the emulator; full-index import still needs its separate
descriptor and resource checks. Further evidence and bounds
are recorded as short labeled entries in `docs/spike/RESULTS.md`.

The new `/notes` top-level route still requires the exact owned loopback origin;
wrong ports and HTTPS remain denied. Normal cookie/CSRF/Fetch-Dest guards stay
in force. The native shell does not forge headers or inject a JavaScript bridge.
The shared client/server use no-store for notes operations; notes/credentials
do not enter the shared query cache, browser storage or navigation URLs.

Record actual Android unlock/save/list/search/GET/lock and recovery flows using
synthetic content, plus locked 423 and unauthenticated 401 behavior, content
clearing after lock/navigation, hostile-ZIM isolation with the vault unlocked,
and loopback-port isolation from another Wi-Fi device. Emulator functional evidence is recorded in RESULTS; physical-phone speed,
removable-media and Wi-Fi/OEM checks remain separate. Do not share credentials,
raw private notes or sensitive DevTools request bodies. The current native,
index, article and map probes remain available.
