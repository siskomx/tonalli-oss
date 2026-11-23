# Isolated LiteRT CPU diagnostic

Isolated SDK compilation, host contract tests, lint and debug APK assembly passed
on 2026-11-12. Main Android/Qwen/Rust project versions remain unchanged. Physical
phone initialization, durable records, short synthetic inference and one
generation cancellation passed later that day; see `docs/spike/RESULTS.md`.
Real English/Spanish retrieval comparison and sustained performance remain open.
The original smoke APK remains at
`target/apks/b12beb3502c095e144d2c4d2e189b591621b065515fc9ddd34e6624a08c3aa8e/app-debug.apk`.
The comparison profile and stage events below pass SDK contract checks,
compilation, lint and APK audit in APK `3c828f1b9513…` (30,847,595 bytes).
Prior phone receipts describe the original APK; comparison phone runs remain open.
Use the existing wrapper from the repository root:

```powershell
rtk proxy android/gradlew.bat --project-dir experiments/litert-android checkContract
```

Observed RED with cached host Kotlin 2.1.21: compile exit 0, test exit 1 at the
deliberate staging stub (2026-11-11). Subsequent cached host compile and actual
staging/lifetime contract tests passed. New off-thread callback-lock tests first
failed, then passed after the native-call/state-lock split. SDK compilation and
Gradle-host contract tests also passed.
Do not build or run this project until the primary
phone probe releases its build/testing window. No model downloader is included.

Candidate: `litert-community/gemma-4-E2B-it-litert-lm` revision
`2d74ac41a33d7da82f39f7d5720a04c68514a103`, file
`gemma-4-E2B-it.litertlm`, 2,588,147,712 bytes, SHA256
`181938105e0eefd105961417e8da75903eacda102c4fce9ce90f50b97139a63c`.
Only an internal checksum-verified `model.litertlm` may be passed to the SDK.
Keep the published file immutable while its engine exists; SDK mmap is expected.

This authorized experiment uses serialized execution windows and a
checksum-verified input. Place the externally obtained trusted file at the probe's
app-specific external `files/incoming/model.litertlm`. The probe copies using
positioned reads, verifies length/hash, and atomically publishes to private
`files/litert-models/<sha256>/model.litertlm`; it never passes the external file to
LiteRT. Run requires a question and retrieved passage; use exactly the same bytes
as the Qwen comparison. Intent extras `question`, `context`, `context_tokens`
(1024/2048/4096; default 2048) permit reproducible fixtures. Launch the common
comparison with explicit `context_tokens=4096`. Output limit is 128. Current
comparison-source sampling is top-k 20, top-p 0.8, temperature 0.7, seed 42,
matching the configured Qwen source profile; actual tokens and native chat
templates/tokenizers can differ. The citation instruction is the same text in
Gemma's system role and Qwen's user prefix. Passage/question bytes stay unchanged;
Qwen alone receives its model-specific `/no_think` suffix. Original smoke evidence
retains its original instruction and greedy sampling profile.

Cancellation explicitly calls native `cancelProcess` on an independent executor;
the worker waits for a terminal callback before disposing conversation/engine.
Native send/cancel operations serialize separately from brief callback-state
bookkeeping. Disposal requires actual terminal state and return of in-flight
native calls. A send exception after possible acceptance retains resources until
an actual callback; `finally` never fabricates terminal state for submitted work.
Initialization has no proven interrupt path. If native work never acknowledges
cancellation, the probe remains busy and retains resources; it does not claim a
deadline or safe forced teardown. Raw callback text/channels and elapsed times
are diagnostic evidence, not token counts or first-useful-answer measurements.
SDK benchmark collection is enabled before engine construction; after a completed
uncancelled request the probe records `getBenchmarkInfo()` and `getTokenCount()`.
The former reports prefill/decode counts, rates, TTFT, initialization time and
effective KV budget. Zero values remain SDK-reported zero/missing evidence; errors
are recorded, never replaced with callback counts. Cancelled conversation state
is discarded without further queries. PSS, thermal samples and useful-answer
timing require external phone observation and raw-output inspection. The first
synthetic phone run returned SDK counts/rates/TTFT; these are distinct from
whole-request time, and do not establish sustained retrieval performance.
The APK bundles the verified SDK's LICENSE
and THIRD_PARTY_NOTICE, plus the repository GPL license.

Each run writes private `files/litert-probe-records/<UUID>.jsonl`, with monotonic
timestamps, raw output/channels, structured SDK-stat fields, errors and terminal
events. Callbacks enqueue bounded records only; the worker flushes them. The queue
holds at most 256 records, each text/channel has a 32,768-character limit, and the
file limit is 2 MiB. At 32 matching run files, a typed capacity refusal preserves
all prior evidence; the probe never automatically deletes it. Each run also has
a durable `<UUID>.status.json` outside the bounded event queue, with cumulative
dropped/truncated/file-limit/write-failure counters. Any omission marks comparison
evidence ineligible; a crash/LMK leaves `completed=false`. Status write failures
are reported and never replaced with success. Initial UID/PID, model SHA, exact
configuration and input hashes are durably flushed before SDK construction/init;
failure to checkpoint refuses inference. The original smoke APK used top-k 1,
top-p 1, temperature 0 and seed 0; the current source records the comparison
sampler above. Initial phone smoke verified durable metadata, output,
terminal status and zero omission counters; cancelled status correctly stays
ineligible. Crash/LMK behavior has not been exercised on the phone.
Screen-on is requested only while active; leaving the activity clears it and
requests sticky cancellation. Context-owned directory aliases are resolved before
checking the probe's child directories; arbitrary child symlinks remain refused.

Pixel 6 Pro CPU initialization and a short cited synthetic answer succeeded;
one in-flight cancellation reached native terminal in about27ms. Sparse PSS and
thermal samples are diagnostic only. Common retrieval quality, sustained speed,
worst-case memory/cancellation and initialization abort remain open.
AAR minSdk 24 differs from its ARM64 ELF API 26
note; this probe requires API 34. ARM64 ELF load alignment and packaged APK ZIP
alignment both passed at 16 KB (`zipalign -c -P 16 4`). Execution on a 16 KB device
still needs separate verification. Packaged permissions are empty, and the APK
contains only the ARM64 JNI library and the required license/notice assets.

Before engine construction, Android reports current `ActivityManager` available
RAM and low-memory status. The temporary conservative admission budget reserves
the full model file bytes plus 1 GiB workspace plus the greater of 1 GiB or the
OS low-memory threshold. This assumption neither extrapolates Google's RSS
numbers nor guarantees runtime fit. Model import/verification runs independently
of this admission and remains permitted when inference is refused.

The notices task resolves an exact nontransitive Maven `@aar`, verifies its
published SHA256, and extracts only LICENSE/THIRD_PARTY_NOTICE. It does not depend
on the ignored feasibility inspection download. The inspected AAR contains no
`aar-metadata.properties`, so it publishes no minCompileSdk value there.

Constructor, initialization, conversation-creation and async-send stage events
use `elapsedRealtimeNanos`. Return events include exact public-call start/end
and wall-duration fields. Stage markers are durably checkpointed beforehand;
marker-to-return spans also include logging overhead and must not be used as
native-call durations. Async send return is not terminal completion. Raw callback
times and SDK-reported `BenchmarkInfo` remain separate: an SDK initialization or
TTFT value must not be assumed to equal an observed wall-clock interval. First
useful answer timing still requires raw-content inspection.
