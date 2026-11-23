# Dependency license record

## Mesh spike Android BLE dependency resolution (2026-11-14)

Optional standalone Android BLE spike only. Exact new Android-reachable
packages, relative to the pre-Mesh lockfile, follow. License texts/source notices
are retained with the build evidence and will accompany the spike artifacts.

| Package | Exact version | License |
|---|---|---|
| bitflags | 1.3.2 | MIT/Apache-2.0 |
| btleplug | 0.12.0 | MIT/Apache-2.0/BSD-3-Clause |
| cesu8 | 1.1.0 | Apache-2.0/MIT |
| combine | 4.6.9 | MIT |
| dashmap | 6.1.0 | MIT |
| futures-executor | 0.3.34 | MIT OR Apache-2.0 |
| futures | 0.3.34 | MIT OR Apache-2.0 |
| hashbrown | 0.14.5 | MIT OR Apache-2.0 |
| jni-sys-macros | 0.4.1 | MIT OR Apache-2.0 |
| jni-sys | 0.3.1 | MIT OR Apache-2.0 |
| jni-sys | 0.4.1 | MIT OR Apache-2.0 |
| jni | 0.19.0 | MIT/Apache-2.0 |
| meshtastic | 0.1.9 | GPL-3.0 |
| mio-serial | 5.0.7 | MIT |
| nix | 0.26.4 | MIT |
| prost-derive | 0.14.6 | Apache-2.0 |
| prost | 0.14.6 | Apache-2.0 |
| serde_bytes | 0.11.19 | MIT OR Apache-2.0 |
| serialport | 4.10.1 | MPL-2.0 |
| signal-hook-registry | 1.4.8 | MIT OR Apache-2.0 |
| thiserror-impl | 1.0.69 | MIT OR Apache-2.0 |
| thiserror | 1.0.69 | MIT OR Apache-2.0 |
| tokio-serial | 5.5.0 | MIT |
| tokio-stream | 0.1.19 | MIT |

cesu8 retains its COPYRIGHT-RUST.txt and README attribution; the package
references the Rust MIT/Apache terms rather than shipping separately named
license files. jni-sys-macros omits its workspace-root license files from the
crate archive; MIT/Apache texts were retrieved at its exact published source
commit64d77b7a5f119d7b55b4e2c169a4668067ff59e6 with URL/hash receipts.
The28 unmodified Java JNI helper files from btleplug0.12.0 are recorded under
third_party/btleplug-0.12.0/UPSTREAM.json with the original component LICENSE.md.
Their Android SDK dependency is the platform API, not a downloaded radio service.
No mock transport, web gateway or normal-app BLE permission is added.

## Mesh spike Windows dependency resolution (2026-11-14)

USB/BLE terminal spike only; not integrated into the normal Android/Windows app.
The user approved the spike, then requested both scenario-selected transports
and no mock transport. Cargo.lock resolves the following new Windows-reachable
packages. Published license texts are retained under the ignored build evidence
folder target/mesh-usb-spike-r1/dependency-licenses and must accompany delivery.
Existing locked package versions remain unchanged. No new dependency is enabled
in default builds. Other-platform-only packages in the lockfile are not validated
by this Windows review; review their exact closure before any Android build.

| Package | Exact version | License |
|---|---|---|
| btleplug | 0.12.0 | MIT/Apache-2.0/BSD-3-Clause |
| dashmap | 6.1.0 | MIT |
| futures-executor | 0.3.34 | MIT OR Apache-2.0 |
| futures | 0.3.34 | MIT OR Apache-2.0 |
| hashbrown | 0.14.5 | MIT OR Apache-2.0 |
| meshtastic | 0.1.9 | GPL-3.0 |
| mio-serial | 5.0.7 | MIT |
| prost-derive | 0.14.6 | Apache-2.0 |
| prost | 0.14.6 | Apache-2.0 |
| serde_bytes | 0.11.19 | MIT OR Apache-2.0 |
| serialport | 4.10.1 | MPL-2.0 |
| tokio-serial | 5.5.0 | MIT |
| tokio-stream | 0.1.19 | MIT |
| windows-collections | 0.3.2 | MIT OR Apache-2.0 |
| windows-core | 0.62.2 | MIT OR Apache-2.0 |
| windows-future | 0.3.2 | MIT OR Apache-2.0 |
| windows-implement | 0.60.2 | MIT OR Apache-2.0 |
| windows-interface | 0.59.3 | MIT OR Apache-2.0 |
| windows-numerics | 0.3.1 | MIT OR Apache-2.0 |
| windows-result | 0.4.1 | MIT OR Apache-2.0 |
| windows-strings | 0.5.1 | MIT OR Apache-2.0 |
| windows-threading | 0.2.1 | MIT OR Apache-2.0 |
| windows | 0.62.2 | MIT OR Apache-2.0 |

GPL-3.0 is treated conservatively as GPL-3.0-only; use Tonalli's GPLv3 option.
Keep the btleplug BSD and inherited MIT/Apache notices separately, as explained
below. Keep serialport MPL file-level source/notices; no incompatible-secondary-
license exception was identified in its packaged license text. MIT/Apache/BSD
components retain their notices. This is a license review, not a hardware claim.

## Proposed Mesh Gateway dependencies (design-time candidates)

Design review only, 2026-11-14. No Cargo dependency/lockfile, Android permission or
Mesh code is added by this review. See
[proposal](docs/2026-11-14-mesh-gateway-design.md) and
[approval-gated tasks](docs/2026-11-14-mesh-gateway-tasks.md).

The published meshtastic0.1.9 archive (SHA256
`181991c1fd7812f0be99f7229daa8c3b4d77acbafd6f640c4d87b4eb4791cf70`)
and its source commit `44016def9e3fefda307223ff759b4c8694bd000b` were inspected.
Candidate license files were read from published archives, not inferred from the
Meshtastic website's branding. The crate's included Cargo.lock informs candidate
versions below; it is not Tonalli's resolved dependency closure.

| Candidate | Exact reviewed version | License evidence / compatibility decision |
|---|---|---|
| [meshtastic](https://docs.rs/crate/meshtastic/0.1.9) | 0.1.9 | Manifest says `GPL-3.0`, archive includes GPLv3 text. Conservatively treat as GPL-3.0-only, not an assumed later-version grant. Compatible with Tonalli's GPLv3 option; retain upstream terms/source/notices. |
| Included Meshtastic protobuf definitions / generated API | Packaged with 0.1.9 | Included protobuf LICENSE is GPLv3. Do not call the definitions Apache merely because protobuf tooling is Apache. Preserve their separate notices; pin archive/source rather than mutable master. |
| [prost](https://docs.rs/crate/prost/0.14.3) | 0.14.3 | Apache-2.0; retain license/notices. Compatible with GPLv3 distribution. |
| [tokio-serial](https://docs.rs/crate/tokio-serial/5.4.5) | 5.4.5 | MIT, published LICENSE; compatible with GPLv3 with notice retained. |
| [serialport](https://docs.rs/crate/serialport/4.9.0) | 4.9.0 | MPL-2.0, published LICENSE.txt; preserve file-level MPL terms and corresponding source. Do not relabel it GPL or discard notices; reject an incompatible-secondary-license exception if found in selected source. |
| [mio-serial](https://docs.rs/crate/mio-serial/5.0.6) | 5.0.6 | MIT, published LICENSE; compatible with GPLv3 with notice retained. |
| [rand](https://docs.rs/crate/rand/0.9.4) | 0.9.4 | MIT OR Apache-2.0, both published license texts inspected; retain selected terms/notices. |
| [btleplug](https://docs.rs/crate/btleplug/0.12.0) (later BLE candidate only) | 0.12.0 | Metadata says `MIT/Apache-2.0/BSD-3-Clause`; actual LICENSE.md covers btleplug under BSD-3-Clause and inherited Rumble portions under MIT/Apache. Preserve component notices, not a blanket invented OR expression. These terms are GPLv3-compatible. Not required for the USB spike. |

Existing Tokio1.54.1/tokio-util0.7.20, axum0.8.9, serde1.0.228,
thiserror2.0.21, rusqlite0.42.0/SQLCipher and other reused dependencies retain
their recorded terms elsewhere in this ledger. Enabling axum's WebSocket feature
will introduce a target-dependent dependency closure that has **not** been added,
resolved or approved here. Android BLE/JNI auxiliary packages likewise require
their own exact-version/source/license review before integration.

After approval and before compiling the spike, record the actual Cargo.lock
closure for the selected target and every newly resolved license/notice. Stop on
unknown or conflicting terms. This provisional review is not a claim that all
future transitive dependencies have already been verified. No upstream examples'
raw logging, personal device selectors or deployment values are copied.

## Standalone country extractor (2026-11-12)

The native helper retains the actual go-pmtiles extraction algorithm used by
NOMAD. It contains no Go HTTP/cloud/server runtime. Pinned source/provenance:
`native/maps-extract/UPSTREAM.json`; module checksums: `go.sum`; retained full
license/notice texts: `native/maps-extract/licenses/` (packaged on both platforms).
Country buffering reuses the recorded Apache-2.0 NOMAD helper. Natural Earth
50m boundaries are public domain, verified against its published terms:
https://www.naturalearthdata.com/about/terms-of-use/.

| Component | Exact version | License |
|---|---|---|
| go-pmtiles algorithm | 1.31.2, a3e4951ea6a0477b784c27c1dcbfd9c130878c5a | BSD-3-Clause |
| Go runtime/toolchain | 1.25.2 | BSD-3-Clause |
| github.com/RoaringBitmap/roaring | 1.5.0 | Apache-2.0 |
| github.com/bits-and-blooms/bitset | 1.2.0 (module graph) | BSD-3-Clause |
| github.com/mschoch/smat | 0.2.0 (module graph) | Apache-2.0 |
| github.com/dustin/go-humanize | 1.0.1 | MIT |
| github.com/paulmach/orb | 0.10.0 | MIT |
| go.mongodb.org/mongo-driver (orb BSON support only) | 1.11.4 | Apache-2.0 |
| golang.org/x/sync | 0.20.0 | BSD-3-Clause |
| github.com/stretchr/testify (retained upstream tests) | 1.11.1 | MIT |
| github.com/davecgh/go-spew (tests) | 1.1.1 | ISC |
| github.com/pmezard/go-difflib (tests) | 1.0.0 | BSD-2-Clause |
| gopkg.in/yaml.v3 (tests) | 3.0.1 | Apache-2.0 (dual license text retained) |
| libloading | 0.8.9 | ISC |


Tonalli is GPL-3.0-or-later. `LICENSE` contains the unmodified GPL-3.0-or-later
text retrieved from the SPDX license-list-data repository:
https://raw.githubusercontent.com/spdx/license-list-data/main/text/GPL-3.0-or-later.txt.
The later-version option is also in `NOTICE` and Cargo package metadata.
The GNU website download failed with a connection reset; SPDX supplied the text.

## Reused Project NOMAD source and chat dependency additions

Compatible chat UI, stream/thinking/abort helpers and tests are adapted from
Crosstalk-Solutions/project-nomad commit
`c903da48c97bc02f5cdfeff2ebcde00a82dfec8e`, Apache-2.0,
Copyright 2024-2026 Crosstalk Solutions LLC. The exact original 10,779-byte
[license](third_party/project-nomad/LICENSE) has SHA256
`e4f125622d3c03c2ebd7caf0eb9f0e9a10029349f75b7e342699cb1a0c212aee`.
[Provenance](third_party/project-nomad/UPSTREAM.json) records original and adapted
file hashes. Each changed file identifies modifications; root NOTICE retains
the original attribution. No conflicting per-file license or upstream NOTICE
was found in the inspected source tree. Jcorp code and evaluation corpus assets
with separate licenses are not copied by this slice.

The shared article-search preparation panel also adapts the compatible PhaseBlock
presentation from `admin/inertia/components/drug-reference/IngestStatus.tsx` at
that revision. Its FDA data/API, estimated timing/progress and raw error text are
replaced by Core-authoritative book preparation and localized messages. This
adds no dependency; the same Apache notice and exact source/adapted hash mapping
cover the changed file.

The following exact new npm resolutions are locked in `web-stub/package-lock.json`.
Installed package versions/declarations and their actual MIT/ISC license texts
were checked; source/integrity and license-file SHA receipts are retained in
`target/nomad-chat-verified-license-texts-r1.json`. The existing UI notice collector
now includes the chat dependency closure and copies license texts verbatim into
built assets, together with the Project NOMAD license/modification list.
React/Inertia/Adonis versions are not downgraded or added as a server dependency.

| Added package | Exact version | Verified license |
| --- | --- | --- |
| `@tabler/icons` | `3.51.0` | MIT |
| `@tabler/icons-react` | `3.36.1` | MIT |
| `@types/debug` | `4.1.13` | MIT |
| `@types/estree-jsx` | `1.0.5` | MIT |
| `@types/hast` | `3.0.5` | MIT |
| `@types/mdast` | `4.0.4` | MIT |
| `@types/ms` | `2.1.0` | MIT |
| `@types/react` | `19.3.0` | MIT |
| `@types/unist` | `2.0.11` | MIT |
| `@types/unist` | `3.0.3` | MIT |
| `@ungap/structured-clone` | `1.4.0` | ISC |
| `bail` | `2.0.2` | MIT |
| `ccount` | `2.0.1` | MIT |
| `character-entities` | `2.0.2` | MIT |
| `character-entities-html4` | `2.1.0` | MIT |
| `character-entities-legacy` | `3.0.0` | MIT |
| `character-reference-invalid` | `2.0.1` | MIT |
| `comma-separated-tokens` | `2.0.3` | MIT |
| `csstype` | `3.2.3` | MIT |
| `debug` | `4.4.3` | MIT |
| `decode-named-character-reference` | `1.3.0` | MIT |
| `dequal` | `2.0.3` | MIT |
| `devlop` | `1.1.0` | MIT |
| `escape-string-regexp` | `5.0.0` | MIT |
| `estree-util-is-identifier-name` | `3.0.0` | MIT |
| `extend` | `3.0.2` | MIT |
| `hast-util-to-jsx-runtime` | `2.3.7` | MIT |
| `hast-util-whitespace` | `3.0.0` | MIT |
| `html-url-attributes` | `3.0.1` | MIT |
| `inline-style-parser` | `0.2.9` | MIT |
| `is-alphabetical` | `2.0.1` | MIT |
| `is-alphanumerical` | `2.0.1` | MIT |
| `is-decimal` | `2.0.1` | MIT |
| `is-hexadecimal` | `2.0.1` | MIT |
| `is-plain-obj` | `4.1.0` | MIT |
| `longest-streak` | `3.1.0` | MIT |
| `markdown-table` | `3.1.0` | MIT |
| `mdast-util-find-and-replace` | `3.0.4` | MIT |
| `mdast-util-from-markdown` | `2.1.0` | MIT |
| `mdast-util-gfm` | `3.1.0` | MIT |
| `mdast-util-gfm-autolink-literal` | `2.1.0` | MIT |
| `mdast-util-gfm-footnote` | `2.1.0` | MIT |
| `mdast-util-gfm-strikethrough` | `2.0.1` | MIT |
| `mdast-util-gfm-table` | `2.0.1` | MIT |
| `mdast-util-gfm-task-list-item` | `2.0.1` | MIT |
| `mdast-util-mdx-expression` | `2.0.1` | MIT |
| `mdast-util-mdx-jsx` | `3.2.1` | MIT |
| `mdast-util-mdxjs-esm` | `2.0.1` | MIT |
| `mdast-util-phrasing` | `4.1.0` | MIT |
| `mdast-util-to-hast` | `13.2.1` | MIT |
| `mdast-util-to-markdown` | `2.2.3` | MIT |
| `mdast-util-to-string` | `4.0.0` | MIT |
| `micromark` | `4.0.3` | MIT |
| `micromark-core-commonmark` | `2.0.4` | MIT |
| `micromark-extension-gfm` | `3.0.0` | MIT |
| `micromark-extension-gfm-autolink-literal` | `2.1.0` | MIT |
| `micromark-extension-gfm-footnote` | `2.1.0` | MIT |
| `micromark-extension-gfm-strikethrough` | `2.1.0` | MIT |
| `micromark-extension-gfm-table` | `2.1.2` | MIT |
| `micromark-extension-gfm-tagfilter` | `2.0.0` | MIT |
| `micromark-extension-gfm-task-list-item` | `2.1.0` | MIT |
| `micromark-factory-destination` | `2.0.1` | MIT |
| `micromark-factory-label` | `2.0.1` | MIT |
| `micromark-factory-space` | `2.1.0` | MIT |
| `micromark-factory-title` | `2.0.1` | MIT |
| `micromark-factory-whitespace` | `2.0.1` | MIT |
| `micromark-util-character` | `2.1.1` | MIT |
| `micromark-util-chunked` | `2.0.1` | MIT |
| `micromark-util-classify-character` | `2.0.1` | MIT |
| `micromark-util-combine-extensions` | `2.0.1` | MIT |
| `micromark-util-decode-numeric-character-reference` | `2.0.2` | MIT |
| `micromark-util-decode-string` | `2.0.1` | MIT |
| `micromark-util-edit-map` | `1.0.0` | MIT |
| `micromark-util-encode` | `2.0.1` | MIT |
| `micromark-util-html-tag-name` | `2.0.1` | MIT |
| `micromark-util-normalize-identifier` | `2.0.1` | MIT |
| `micromark-util-resolve-all` | `2.0.1` | MIT |
| `micromark-util-sanitize-uri` | `2.0.1` | MIT |
| `micromark-util-subtokenize` | `2.1.0` | MIT |
| `micromark-util-symbol` | `2.0.1` | MIT |
| `micromark-util-types` | `2.0.3` | MIT |
| `ms` | `2.1.3` | MIT |
| `parse-entities` | `4.0.2` | MIT |
| `property-information` | `7.2.0` | MIT |
| `react-markdown` | `10.1.0` | MIT |
| `remark-gfm` | `4.0.1` | MIT |
| `remark-parse` | `11.0.0` | MIT |
| `remark-rehype` | `11.1.2` | MIT |
| `remark-stringify` | `11.0.0` | MIT |
| `space-separated-tokens` | `2.0.2` | MIT |
| `stringify-entities` | `4.0.4` | MIT |
| `style-to-object` | `2.0.2` | MIT |
| `trim-lines` | `3.0.1` | MIT |
| `trough` | `2.2.0` | MIT |
| `unified` | `11.0.5` | MIT |
| `unist-util-is` | `6.0.1` | MIT |
| `unist-util-position` | `5.0.0` | MIT |
| `unist-util-stringify-position` | `4.0.0` | MIT |
| `unist-util-visit` | `5.2.0` | MIT |
| `unist-util-visit-parents` | `6.1.0` | MIT |
| `vfile` | `6.0.3` | MIT |
| `vfile-message` | `4.0.4` | MIT |
| `zwitch` | `2.0.4` | MIT |

## Task 11 optional AI inputs

### Isolated LiteRT experiment inputs (not the main app)

The user authorized one bounded optional CPU comparison. Published direct SDK
coordinate is `com.google.ai.edge.litertlm:litertlm-android:0.19.1`, Apache-2.0,
from [Google Maven](https://dl.google.com/dl/android/maven2/com/google/ai/edge/litertlm/litertlm-android/0.19.1/litertlm-android-0.19.1.pom).
The 21,200,360-byte AAR was independently hashed as
`341f66349ac962e0b6733b8da419b045a4834fca2f7db0bd13e049ce1cfcca72`, matching
the publisher sidecar. Its Apache LICENSE and 2,250,200-byte THIRD_PARTY_NOTICE
are retained in the assembled probe APK. The notice is not an exact-version ledger
for every bundled native component; that part of the audit remains incomplete.
The isolated SDK compilation, host contract tests, lint and APK assembly passed
on 2026-11-12. The actual `debugRuntimeClasspath` is frozen in
`experiments/litert-android/gradle.lockfile`. Cached publisher POM license
declarations were inspected for each resolved coordinate below; their receipt is
`target/gemma-litert-feasibility/runtime-dependencies.json`. The coroutines BOM
and multiplatform root are graph metadata, not additional native libraries.
No main-project dependency or compiler was changed.

| Resolved isolated runtime coordinate | Publisher POM license |
| --- | --- |
| `com.google.ai.edge.litertlm:litertlm-android:0.19.1` | Apache-2.0 |
| `com.google.code.gson:gson:2.14.0` | Apache-2.0 |
| `com.google.errorprone:error_prone_annotations:2.48.0` | Apache-2.0 |
| `org.jetbrains.kotlin:kotlin-reflect:2.4.0` | Apache-2.0 |
| `org.jetbrains.kotlin:kotlin-stdlib:2.4.0` | Apache-2.0 |
| `org.jetbrains.kotlinx:kotlinx-coroutines-android:1.11.0` | Apache-2.0 |
| `org.jetbrains.kotlinx:kotlinx-coroutines-core-jvm:1.11.0` | Apache-2.0 |
| `org.jetbrains.kotlinx:kotlinx-coroutines-core:1.11.0` | Apache-2.0 |
| `org.jetbrains.kotlinx:kotlinx-coroutines-bom:1.11.0` | Apache-2.0 |
| `org.jetbrains:annotations:23.0.0` | Apache-2.0 |

Build-only Kotlin Gradle plugin/compiler 2.4.0 are isolated here, with the
existing AGP 8.7.2 and Gradle 8.9 wrapper. The SDK's composite native notice is
bundled, but exact versions/license closure for every bundled native component
remain unverified; the JVM table does not close that gap.

Bounded publisher-provenance lookup (2026-11-12): the official
[v0.19.1 release](https://github.com/google-ai-edge/LiteRT-LM/releases/tag/v0.19.1)
resolves to `7a03724cece417f1724c923b3d527b4319730841`; its WORKSPACE contains
declared source dependency pins. Checked release assets/metadata and the AAR
digest's attestation lookup did not establish a build-provenance link from the
Maven binary to that commit. Those pins therefore remain source declarations,
not verified versions linked into this AAR. Evidence:
`target/gemma-litert-feasibility/provenance-lookup-r1/findings.json`.

Pinned experimental model `litert-community/gemma-4-E2B-it-litert-lm` revision
`2d74ac41a33d7da82f39f7d5720a04c68514a103` declares Apache-2.0. Exact model
bytes/SHA/URL are recorded in packs.toml; the checked downloader verified the
complete file SHA before atomic activation on2026-11-12. Runtime remains untested.
[Google's deployment guide](https://developers.google.com/edge/litert-lm/models/gemma-4)
links the deployment repository, and the
[immutable publisher metadata](https://huggingface.co/api/models/litert-community/gemma-4-E2B-it-litert-lm/revision/2d74ac41a33d7da82f39f7d5720a04c68514a103?blobs=true)
supplies the file pin. Gemma3n was not selected; its access terms are different
and must not be inferred from Gemma4's license.

### Existing Rust/llama.cpp path

The optional `ai` feature selects the local `llama-cpp-2` 0.1.161 wrapper fork.
Native delivery notices are retained in
`android/licenses/llama-cpp-runtime-NOTICES.txt`: the wrapper's MIT/Apache texts,
the pinned llama.cpp MIT text and Mozilla SGEMM's source copyright/permission.
Android's existing notice packager includes this file; AI Windows bundles include
the same file. This does not enable any previously disabled native target.
Its upstream is `utilityai/llama-cpp-rs` commit
`e7af607ebd5ec2932709ef918c17cca2f9b0fce4`; both pinned upstream MIT and Apache-2.0
texts are retained in `crates/llama-cpp-2-fork/`. The wrapper/sys declaration is
MIT OR Apache-2.0. The registry sys crate is exactly 0.1.161 and vendors native
llama.cpp `d81235049384534c167caea52b85a694f6103d14` (MIT, ggml authors 2023–2026).
The fork adds owned CPU cancellation and a safe resolved-pooling getter, and disables implicit sys defaults; it
preserves explicit common/Metal feature forwarding. It is excluded from default
workspace members so an ordinary non-AI build does not compile the C++ backend.

Locked metadata v2 resolves 367 registry packages plus the local wrapper. These
new exact versions supplement the earlier tables. License strings below are
upstream manifest declarations; `/` denotes historical alternative syntax.

| Package | Exact version | Declared license |
| --- | --- | --- |
| bindgen | 0.72.1 | BSD-3-Clause |
| cexpr | 0.6.0 | Apache-2.0/MIT |
| clang-sys | 1.9.1 | Apache-2.0 |
| cmake | 0.1.58 | MIT OR Apache-2.0 |
| enumflags2 | 0.7.12 | MIT OR Apache-2.0 |
| enumflags2_derive | 0.7.12 | MIT OR Apache-2.0 |
| find_cuda_helper | 0.2.0 | MIT OR Apache-2.0 |
| itertools | 0.13.0 | MIT OR Apache-2.0 |
| libloading | 0.8.9 | ISC |
| llama-cpp-2 (local wrapper) | 0.1.161 | MIT OR Apache-2.0 |
| llama-cpp-sys-2 | 0.1.161 | MIT OR Apache-2.0 |
| prettyplease | 0.2.37 | MIT OR Apache-2.0 |
| same-file | 1.0.6 | Unlicense/MIT |
| shlex | 1.3.0 | MIT OR Apache-2.0 |
| tracing-attributes | 0.1.31 | MIT |
| valuable | 0.1.1 | MIT |
| walkdir | 2.5.0 | Unlicense/MIT |
| winapi-util | 0.1.11 | Unlicense OR MIT |

The excluded wrapper's standalone development lock additionally selects these
test dependencies; they are not additions to the app's runtime graph:

| Package | Exact version | Declared license |
| --- | --- | --- |
| core_detect | 1.0.0 | MIT/Apache-2.0 |
| encoding_rs | 0.8.42 | (Apache-2.0 OR MIT) AND BSD-3-Clause |
| multiversion_no_op | 1.0.0 | Apache-2.0 OR MIT |
| nu-ansi-term | 0.50.3 | MIT |
| sharded-slab | 0.1.7 | MIT |
| simdutf8 | 0.1.5 | MIT OR Apache-2.0 |
| static_assertions | 1.1.0 | MIT OR Apache-2.0 |
| thread_local | 1.1.10 | MIT OR Apache-2.0 |
| tracing-log | 0.2.0 | MIT |
| tracing-serde | 0.2.0 | MIT |
| tracing-subscriber | 0.3.23 | MIT |

Native Windows CMake evidence confirms CPU/static, common OFF, OpenMP OFF,
native-host tuning OFF and Metal OFF. Unfiltered Cargo metadata also describes
the conditional Mac Metal edge; it is not evidence of a Windows GPU build.
The retained upstream native source bundle is not uniformly MIT: common uses
cpp-httplib 0.59.0 (MIT), nlohmann JSON 3.12.0 (MIT with inline attributions),
sheredom subprocess (Unlicense), and base64.hpp (Unlicense). Those common targets
are disabled here. Optional mtmd uses BSD-2-Clause xxhash, public-domain SHA-1/
SHA-256, MIT rotate-bits, MIT OR Unlicense stb_image and Unlicense OR MIT-0
miniaudio; mtmd is disabled. Mozilla llamafile SGEMM is MIT and is explicitly
disabled by the Android build script; desktop inclusion must follow actual
build evidence. KleidiAI defaults off. These are source-bundle inventory facts,
not a claim that every bundled component is linked.

Model candidates are separately licensed data: Qwen3 1.7B/4B Q4_K_M and 0.6B Q4_0
Apache-2.0 (0.6B: ggml-org/Qwen3-0.6B-GGUF revision afe8ad3577aa, 428,970,080 B,
SHA256 da2572f1…417d4, added to the signed catalog at revision 3 for 4 GB phones),
BGE-small-en-v1.5 Q8_0 MIT. Immutable revisions, publishers, sizes and expected
SHA256 are in `docs/spike/packs.toml`; local verification is recorded separately.
Initial evidence: `target/task11-license-inventory-privacy-review.json`,
`ai-cargo-metadata-v2.json`, `ai-fork-standalone-delta.json` and the fork's
`UPSTREAM.toml`. Android linkage/static-C++ notices and final binary notice
closure remain pending the actual AI-enabled Android build.

## Added in Task 1

Direct dependencies are pinned in Cargo.toml; the complete resolution is pinned
in Cargo.lock, including registry checksums. On 2026-11-11 the following exact
versions, upstreams and license declarations were checked using
`cargo metadata --format-version 1 --locked --offline` against downloaded
crate manifests. This records upstream declarations, not a full license-file
or compatibility audit. It includes target-specific dependencies that are
resolved but not necessarily compiled on the tested targets. CLI TLS uses
Rustls with ring, which includes native cryptographic code. The native core
smoke-build evidence and its limits are in
[native-gate-validation.md](docs/spike/native-gate-validation.md); Android and
other target builds of the downloader remain untested.

Runtime direct dependencies: sha2 0.10.9, thiserror 2.0.21, rusqlite 0.42.0
(default features disabled; bundled-sqlcipher-vendored-openssl), tantivy 0.27.0,
zstd 0.14.3, xz2 0.1.7 (static), memmap2 0.9.12, unicode-normalization 0.1.26,
tempfile 3.23.0 and the vendored zim 0.5.0 fork (core); reqwest 0.12.24
(blocking + Rustls, default features disabled), serde 1.0.228, toml 1.2.1+spec-1.1.0,
sha2 0.10.9, fs2 0.4.3 and tempfile 3.23.0 (CLI). Core tests also use tempfile.

The catalog group also uses the already-recorded exact ring0.17.14, url2.5.8 and
fs2 0.4.3 directly in Core. Publisher CLI tooling uses ring0.17.14 and
zeroize1.9.1 directly. These add dependency edges, not new package versions;
their verified exact license rows remain in the table below.

The shared downloader promotes the existing reqwest0.12.24 blocking/Rustls
configuration into Core and adds a direct toml1.2.1+spec-1.1.0 edge for compatible resume
metadata. The CLI delegates to that same engine. The compatibility reconciliation
uses the already-locked toml 1.2.1+spec-1.1.0 (MIT OR Apache-2.0), whose default
parse/display/serde APIs retain the existing manifest/resume semantics.
This removes the obsolete root-only toml 0.8.23, toml_datetime 0.6.11,
toml_edit 0.22.27, toml_write 0.1.2, serde_spanned 0.6.9 and winnow 0.7.15
resolution. Their older ledger rows below are retained for historical artifacts.
The separate desktop GTK closure keeps toml 0.8.2 / toml_datetime 0.6.3;
desktop's newly resolved downloader coordinates match the existing root lock.
No newly introduced package version or license is required by this reconciliation.

For the Windows interface-binding adapter, pinned source baselines for
reqwest0.12.24 (MIT OR Apache-2.0) and hyper-util0.1.21 (MIT) are vendored under
`crates/reqwest-fork/` and `crates/hyper-util-fork/`. The desktop application now
selects these audited forks through desktop-only Cargo patches, replacing the
registry sources at the same exact versions; the root application graph remains
unchanged. Each UPSTREAM.json
records official archive/VCS/version, original per-file hashes, verbatim license
hashes and seven changed/new files with reasons/current hashes. The narrow
Windows-only socket-binding patch adds features to the already locked
windows-sys0.61.2 (MIT OR Apache-2.0); no new package coordinate is introduced.
Upstream notices remain intact. Offline desktop metadata confirms only these two
source replacements, with no other package coordinates or versions changed.
Desktop default-app tests, strict checks and release compilation passed. Actual
download/install/reopen acceptance remains a separate runtime check.

The table was refreshed from locked Cargo metadata after Task 3's reader and
folded-title dependencies were added (269 registry packages). It includes target-specific
packages; resolution does not establish a successful build on those targets.
Task 2 promotes the already-resolved memmap2 0.9.12 to a direct dependency; its
MIT OR Apache-2.0 declaration is recorded below. Windows tests and the Android
ARM64/API 34 release rebuild passed with the new PackSource implementation.

| Dependency | Exact version | Declared license | Source |
| --- | --- | --- | --- |
| aho-corasick | 1.1.5 | Unlicense OR MIT | [upstream](https://github.com/BurntSushi/aho-corasick) |
| arc-swap | 1.9.5 | MIT OR Apache-2.0 | [upstream](https://github.com/vorner/arc-swap) |
| async-trait | 0.1.92 | MIT OR Apache-2.0 | [upstream](https://github.com/dtolnay/async-trait) |
| atomic-waker | 1.1.2 | Apache-2.0 OR MIT | [upstream](https://github.com/smol-rs/atomic-waker) |
| autocfg | 1.5.1 | Apache-2.0 OR MIT | [upstream](https://github.com/cuviper/autocfg) |
| base64 | 0.22.1 | MIT OR Apache-2.0 | [upstream](https://github.com/marshallpierce/rust-base64) |
| base64 | 0.23.1 | MIT OR Apache-2.0 | [upstream](https://github.com/marshallpierce/rust-base64) |
| bitflags | 2.13.2 | MIT OR Apache-2.0 | [upstream](https://github.com/bitflags/bitflags) |
| bitpacking | 0.9.3 | MIT | [upstream](https://github.com/quickwit-oss/bitpacking) |
| bitreader | 0.3.11 | MIT OR Apache-2.0 | [upstream](https://github.com/irauta/bitreader) |
| block-buffer | 0.10.4 | MIT OR Apache-2.0 | [upstream](https://github.com/RustCrypto/utils) |
| block-buffer | 0.12.1 | MIT OR Apache-2.0 | [upstream](https://github.com/RustCrypto/utils) |
| bon | 3.10.3 | MIT OR Apache-2.0 | [upstream](https://github.com/elastio/bon) |
| bon-macros | 3.10.3 | MIT OR Apache-2.0 | [upstream](https://github.com/elastio/bon) |
| bumpalo | 3.20.3 | MIT OR Apache-2.0 | [upstream](https://github.com/fitzgen/bumpalo) |
| byteorder | 1.5.0 | Unlicense OR MIT | [upstream](https://github.com/BurntSushi/byteorder) |
| bytes | 1.12.1 | MIT | [upstream](https://github.com/tokio-rs/bytes) |
| cc | 1.11.1 | MIT OR Apache-2.0 | [upstream](https://github.com/rust-lang/cc-rs) |
| census | 0.4.2 | MIT | [upstream](https://github.com/quickwit-inc/census) |
| cfg-if | 1.0.5 | MIT OR Apache-2.0 | [upstream](https://github.com/rust-lang/cfg-if) |
| cfg_aliases | 0.2.2 | MIT | [upstream](https://github.com/katharostech/cfg_aliases) |
| chacha20 | 0.10.2 | MIT OR Apache-2.0 | [upstream](https://github.com/RustCrypto/stream-ciphers) |
| const-oid | 0.10.3 | Apache-2.0 OR MIT | [upstream](https://github.com/RustCrypto/formats) |
| cpufeatures | 0.2.17 | MIT OR Apache-2.0 | [upstream](https://github.com/RustCrypto/utils) |
| cpufeatures | 0.3.1 | MIT OR Apache-2.0 | [upstream](https://github.com/RustCrypto/utils) |
| crc32fast | 1.5.5 | MIT OR Apache-2.0 | [upstream](https://github.com/srijs/rust-crc32fast) |
| crossbeam-channel | 0.5.17 | MIT OR Apache-2.0 | [upstream](https://github.com/crossbeam-rs/crossbeam) |
| crossbeam-deque | 0.8.8 | MIT OR Apache-2.0 | [upstream](https://github.com/crossbeam-rs/crossbeam) |
| crossbeam-epoch | 0.9.21 | MIT OR Apache-2.0 | [upstream](https://github.com/crossbeam-rs/crossbeam) |
| crossbeam-utils | 0.8.23 | MIT OR Apache-2.0 | [upstream](https://github.com/crossbeam-rs/crossbeam) |
| crunchy | 0.2.4 | MIT | [upstream](https://github.com/eira-fransham/crunchy) |
| crypto-common | 0.1.7 | MIT OR Apache-2.0 | [upstream](https://github.com/RustCrypto/traits) |
| crypto-common | 0.2.2 | MIT OR Apache-2.0 | [upstream](https://github.com/RustCrypto/traits) |
| darling | 0.24.1 | MIT | [upstream](https://github.com/TedDriggs/darling) |
| darling_core | 0.24.1 | MIT | [upstream](https://github.com/TedDriggs/darling) |
| darling_macro | 0.24.1 | MIT | [upstream](https://github.com/TedDriggs/darling) |
| datasketches | 0.5.0 | Apache-2.0 | [upstream](https://github.com/apache/datasketches-rust) |
| deranged | 0.5.9 | MIT OR Apache-2.0 | [upstream](https://github.com/jhpratt/deranged) |
| digest | 0.10.7 | MIT OR Apache-2.0 | [upstream](https://github.com/RustCrypto/traits) |
| digest | 0.11.3 | MIT OR Apache-2.0 | [upstream](https://github.com/RustCrypto/traits) |
| displaydoc | 0.2.7 | MIT OR Apache-2.0 | [upstream](https://github.com/yaahc/displaydoc) |
| downcast-rs | 2.0.2 | MIT OR Apache-2.0 | [upstream](https://github.com/marcianx/downcast-rs) |
| either | 1.19.0 | MIT OR Apache-2.0 | [upstream](https://github.com/rayon-rs/either) |
| equivalent | 1.0.3 | Apache-2.0 OR MIT | [upstream](https://github.com/indexmap-rs/equivalent) |
| erased-serde | 0.4.11 | MIT OR Apache-2.0 | [upstream](https://github.com/dtolnay/erased-serde) |
| errno | 0.3.14 | MIT OR Apache-2.0 | [upstream](https://github.com/lambda-fairy/rust-errno) |
| fallible-iterator | 0.3.0 | MIT/Apache-2.0 | [upstream](https://github.com/sfackler/rust-fallible-iterator) |
| fallible-streaming-iterator | 0.1.9 | MIT/Apache-2.0 | [upstream](https://github.com/sfackler/fallible-streaming-iterator) |
| fastdivide | 0.4.2 | zlib-acknowledgement OR MIT | [upstream](https://github.com/fulmicoton/fastdivide) |
| fastrand | 2.5.0 | Apache-2.0 OR MIT | [upstream](https://github.com/smol-rs/fastrand) |
| find-msvc-tools | 0.1.15 | MIT OR Apache-2.0 | [upstream](https://github.com/rust-lang/cc-rs) |
| fnv | 1.0.7 | Apache-2.0 / MIT | [upstream](https://github.com/servo/rust-fnv) |
| foldhash | 0.2.0 | Zlib | [upstream](https://github.com/orlp/foldhash) |
| form_urlencoded | 1.2.2 | MIT OR Apache-2.0 | [upstream](https://github.com/servo/rust-url) |
| frostem | 1.20260821.8 | BSD-3-Clause | [upstream](https://github.com/Xuanwo/frostem) |
| fs2 | 0.4.3 | MIT/Apache-2.0 | [upstream](https://github.com/danburkert/fs2-rs) |
| fs4 | 0.13.1 | MIT OR Apache-2.0 | [upstream](https://github.com/al8n/fs4-rs) |
| futures-channel | 0.3.34 | MIT OR Apache-2.0 | [upstream](https://github.com/rust-lang/futures-rs) |
| futures-core | 0.3.34 | MIT OR Apache-2.0 | [upstream](https://github.com/rust-lang/futures-rs) |
| futures-io | 0.3.34 | MIT OR Apache-2.0 | [upstream](https://github.com/rust-lang/futures-rs) |
| futures-macro | 0.3.34 | MIT OR Apache-2.0 | [upstream](https://github.com/rust-lang/futures-rs) |
| futures-sink | 0.3.34 | MIT OR Apache-2.0 | [upstream](https://github.com/rust-lang/futures-rs) |
| futures-task | 0.3.34 | MIT OR Apache-2.0 | [upstream](https://github.com/rust-lang/futures-rs) |
| futures-util | 0.3.34 | MIT OR Apache-2.0 | [upstream](https://github.com/rust-lang/futures-rs) |
| generic-array | 0.14.7 | MIT | [upstream](https://github.com/fizyk20/generic-array.git) |
| getrandom | 0.2.17 | MIT OR Apache-2.0 | [upstream](https://github.com/rust-random/getrandom) |
| getrandom | 0.3.4 | MIT OR Apache-2.0 | [upstream](https://github.com/rust-random/getrandom) |
| getrandom | 0.4.3 | MIT OR Apache-2.0 | [upstream](https://github.com/rust-random/getrandom) |
| hashbrown | 0.17.1 | MIT OR Apache-2.0 | [upstream](https://github.com/rust-lang/hashbrown) |
| htmlescape | 0.3.1 | Apache-2.0 / MIT / MPL-2.0 | [upstream](https://github.com/veddan/rust-htmlescape) |
| http | 1.5.0 | MIT OR Apache-2.0 | [upstream](https://github.com/hyperium/http) |
| http-body | 1.1.0 | MIT | [upstream](https://github.com/hyperium/http-body) |
| http-body-util | 0.1.5 | MIT | [upstream](https://github.com/hyperium/http-body) |
| httparse | 1.10.1 | MIT OR Apache-2.0 | [upstream](https://github.com/seanmonstar/httparse) |
| hybrid-array | 0.4.15 | MIT OR Apache-2.0 | [upstream](https://github.com/RustCrypto/hybrid-array) |
| hyper | 1.12.0 | MIT | [upstream](https://github.com/hyperium/hyper) |
| hyper-rustls | 0.27.10 | Apache-2.0 OR ISC OR MIT | [upstream](https://github.com/rustls/hyper-rustls) |
| hyper-util | 0.1.21 | MIT | [upstream](https://github.com/hyperium/hyper-util) |
| icu_collections | 2.4.0 | Unicode-3.0 | [upstream](https://github.com/unicode-org/icu4x) |
| icu_locale_core | 2.4.0 | Unicode-3.0 | [upstream](https://github.com/unicode-org/icu4x) |
| icu_normalizer | 2.4.0 | Unicode-3.0 | [upstream](https://github.com/unicode-org/icu4x) |
| icu_normalizer_data | 2.4.0 | Unicode-3.0 | [upstream](https://github.com/unicode-org/icu4x) |
| icu_properties | 2.4.0 | Unicode-3.0 | [upstream](https://github.com/unicode-org/icu4x) |
| icu_properties_data | 2.4.0 | Unicode-3.0 | [upstream](https://github.com/unicode-org/icu4x) |
| icu_provider | 2.4.0 | Unicode-3.0 | [upstream](https://github.com/unicode-org/icu4x) |
| ident_case | 1.0.1 | MIT/Apache-2.0 | [upstream](https://github.com/TedDriggs/ident_case) |
| idna | 1.1.0 | MIT OR Apache-2.0 | [upstream](https://github.com/servo/rust-url/) |
| idna_adapter | 1.2.2 | Apache-2.0 OR MIT | [upstream](https://github.com/hsivonen/idna_adapter) |
| indexmap | 2.14.2 | Apache-2.0 OR MIT | [upstream](https://github.com/indexmap-rs/indexmap) |
| inventory | 0.3.25 | MIT OR Apache-2.0 | [upstream](https://github.com/dtolnay/inventory) |
| ipnet | 2.12.2 | MIT OR Apache-2.0 | [upstream](https://github.com/krisprice/ipnet) |
| itertools | 0.14.0 | MIT OR Apache-2.0 | [upstream](https://github.com/rust-itertools/itertools) |
| itoa | 1.0.19 | MIT OR Apache-2.0 | [upstream](https://github.com/dtolnay/itoa) |
| jobserver | 0.1.35 | MIT OR Apache-2.0 | [upstream](https://github.com/rust-lang/jobserver-rs) |
| js-sys | 0.3.107 | MIT OR Apache-2.0 | [upstream](https://github.com/wasm-bindgen/wasm-bindgen/tree/master/crates/js-sys) |
| levenshtein_automata | 0.2.1 | MIT | [upstream](https://github.com/tantivy-search/levenshtein-automata) |
| libc | 0.2.191 | MIT OR Apache-2.0 | [upstream](https://github.com/rust-lang/libc) |
| libsqlite3-sys | 0.40.0 | MIT | [upstream](https://github.com/rusqlite/rusqlite) |
| linux-raw-sys | 0.12.1 | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT | [upstream](https://github.com/sunfishcode/linux-raw-sys) |
| litemap | 0.8.3 | Unicode-3.0 | [upstream](https://github.com/unicode-org/icu4x) |
| log | 0.4.35 | MIT OR Apache-2.0 | [upstream](https://github.com/rust-lang/log) |
| lru | 0.18.5 | MIT | [upstream](https://github.com/jeromefroe/lru-rs.git) |
| lru-slab | 0.1.3 | MIT OR Apache-2.0 OR Zlib | [upstream](https://github.com/Ralith/lru-slab) |
| lz4_flex | 0.14.0 | MIT | [upstream](https://github.com/pseitz/lz4_flex) |
| lzma-sys | 0.1.20 | MIT/Apache-2.0 | [upstream](https://github.com/alexcrichton/xz2-rs) |
| md-5 | 0.11.0 | MIT OR Apache-2.0 | [upstream](https://github.com/RustCrypto/hashes) |
| measure_time | 0.9.0 | MIT | [upstream](https://github.com/PSeitz/rust_measure_time) |
| memchr | 2.8.3 | Unlicense OR MIT | [upstream](https://github.com/BurntSushi/memchr) |
| memmap2 | 0.9.12 | MIT OR Apache-2.0 | [upstream](https://github.com/RazrFalcon/memmap2-rs) |
| minimal-lexical | 0.2.1 | MIT/Apache-2.0 | [upstream](https://github.com/Alexhuszagh/minimal-lexical) |
| mio | 1.2.4 | MIT | [upstream](https://github.com/tokio-rs/mio) |
| murmurhash32 | 0.4.0 | MIT | [upstream](https://github.com/quickwit-inc/murmurhash32) |
| nom | 7.1.3 | MIT | [upstream](https://github.com/Geal/nom) |
| num-conv | 0.2.2 | MIT OR Apache-2.0 | [upstream](https://github.com/jhpratt/num-conv) |
| num-traits | 0.2.19 | MIT OR Apache-2.0 | [upstream](https://github.com/rust-num/num-traits) |
| once_cell | 1.21.4 | MIT OR Apache-2.0 | [upstream](https://github.com/matklad/once_cell) |
| oneshot | 0.1.13 | MIT OR Apache-2.0 | [upstream](https://github.com/faern/oneshot) |
| openssl-src | 300.6.1+3.6.3 | MIT/Apache-2.0 | [upstream](https://github.com/alexcrichton/openssl-src-rs) |
| openssl-sys | 0.9.118 | MIT | [upstream](https://github.com/rust-openssl/rust-openssl) |
| ordered-float | 5.5.1 | MIT | [upstream](https://github.com/reem/rust-ordered-float) |
| ownedbytes | 0.9.0 | MIT | [upstream](https://github.com/quickwit-oss/tantivy) |
| percent-encoding | 2.3.2 | MIT OR Apache-2.0 | [upstream](https://github.com/servo/rust-url/) |
| pin-project-lite | 0.2.17 | Apache-2.0 OR MIT | [upstream](https://github.com/taiki-e/pin-project-lite) |
| pkg-config | 0.3.34 | MIT OR Apache-2.0 | [upstream](https://github.com/rust-lang/pkg-config-rs) |
| potential_utf | 0.1.6 | Unicode-3.0 | [upstream](https://github.com/unicode-org/icu4x) |
| powerfmt | 0.2.1 | MIT OR Apache-2.0 | [upstream](https://github.com/jhpratt/powerfmt) |
| ppv-lite86 | 0.2.21 | MIT OR Apache-2.0 | [upstream](https://github.com/cryptocorrosion/cryptocorrosion) |
| prettyplease | 0.3.0 | MIT OR Apache-2.0 | [upstream](https://github.com/dtolnay/prettyplease) |
| proc-macro2 | 1.0.107 | MIT OR Apache-2.0 | [upstream](https://github.com/dtolnay/proc-macro2) |
| quinn | 0.11.12 | MIT OR Apache-2.0 | [upstream](https://github.com/quinn-rs/quinn) |
| quinn-proto | 0.11.19 | MIT OR Apache-2.0 | [upstream](https://github.com/quinn-rs/quinn) |
| quinn-udp | 0.5.16 | MIT OR Apache-2.0 | [upstream](https://github.com/quinn-rs/quinn) |
| quote | 1.0.47 | MIT OR Apache-2.0 | [upstream](https://github.com/dtolnay/quote) |
| r-efi | 5.3.0 | MIT OR Apache-2.0 OR LGPL-2.1-or-later | [upstream](https://github.com/r-efi/r-efi) |
| r-efi | 6.0.0 | MIT OR Apache-2.0 OR LGPL-2.1-or-later | [upstream](https://github.com/r-efi/r-efi) |
| rand | 0.10.3 | MIT OR Apache-2.0 | [upstream](https://github.com/rust-random/rand) |
| rand | 0.9.5 | MIT OR Apache-2.0 | [upstream](https://github.com/rust-random/rand) |
| rand_chacha | 0.9.0 | MIT OR Apache-2.0 | [upstream](https://github.com/rust-random/rand) |
| rand_core | 0.10.1 | MIT OR Apache-2.0 | [upstream](https://github.com/rust-random/rand_core) |
| rand_core | 0.9.5 | MIT OR Apache-2.0 | [upstream](https://github.com/rust-random/rand) |
| rand_pcg | 0.10.2 | MIT OR Apache-2.0 | [upstream](https://github.com/rust-random/rngs) |
| rayon | 1.12.0 | MIT OR Apache-2.0 | [upstream](https://github.com/rayon-rs/rayon) |
| rayon-core | 1.13.0 | MIT OR Apache-2.0 | [upstream](https://github.com/rayon-rs/rayon) |
| regex | 1.13.1 | MIT OR Apache-2.0 | [upstream](https://github.com/rust-lang/regex) |
| regex-automata | 0.4.18 | MIT OR Apache-2.0 | [upstream](https://github.com/rust-lang/regex) |
| regex-syntax | 0.8.11 | MIT OR Apache-2.0 | [upstream](https://github.com/rust-lang/regex) |
| reqwest | 0.12.24 | MIT OR Apache-2.0 | [upstream](https://github.com/seanmonstar/reqwest) |
| ring | 0.17.14 | Apache-2.0 AND ISC | [upstream](https://github.com/briansmith/ring) |
| rsqlite-vfs | 0.2.0 | MIT | [upstream](https://github.com/Spxg/sqlite-wasm-rs) |
| rusqlite | 0.42.0 | MIT | [upstream](https://github.com/rusqlite/rusqlite) |
| rustc-hash | 2.1.3 | Apache-2.0 OR MIT | [upstream](https://github.com/rust-lang/rustc-hash) |
| rustix | 1.1.6 | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT | [upstream](https://github.com/bytecodealliance/rustix) |
| rustls | 0.23.46 | Apache-2.0 OR ISC OR MIT | [upstream](https://github.com/rustls/rustls) |
| rustls-pki-types | 1.16.0 | MIT OR Apache-2.0 | [upstream](https://github.com/rustls/pki-types) |
| rustls-webpki | 0.103.16 | ISC | [upstream](https://github.com/rustls/webpki) |
| rustversion | 1.0.23 | MIT OR Apache-2.0 | [upstream](https://github.com/dtolnay/rustversion) |
| ryu | 1.0.23 | Apache-2.0 OR BSL-1.0 | [upstream](https://github.com/dtolnay/ryu) |
| serde | 1.0.228 | MIT OR Apache-2.0 | [upstream](https://github.com/serde-rs/serde) |
| serde-tuple-vec-map | 1.0.1 | MIT | [upstream](https://github.com/daboross/serde-tuple-vec-map/) |
| serde_core | 1.0.228 | MIT OR Apache-2.0 | [upstream](https://github.com/serde-rs/serde) |
| serde_derive | 1.0.228 | MIT OR Apache-2.0 | [upstream](https://github.com/serde-rs/serde) |
| serde_json | 1.0.154 | MIT OR Apache-2.0 | [upstream](https://github.com/serde-rs/json) |
| serde_spanned | 0.6.9 | MIT OR Apache-2.0 | [upstream](https://github.com/toml-rs/toml) |
| serde_urlencoded | 0.7.1 | MIT/Apache-2.0 | [upstream](https://github.com/nox/serde_urlencoded) |
| sha2 | 0.10.9 | MIT OR Apache-2.0 | [upstream](https://github.com/RustCrypto/hashes) |
| shlex | 2.0.1 | MIT OR Apache-2.0 | [upstream](https://github.com/comex/rust-shlex) |
| sketches-ddsketch | 0.4.2 | Apache-2.0 | [upstream](https://github.com/mheffner/rust-sketches-ddsketch) |
| slab | 0.4.12 | MIT | [upstream](https://github.com/tokio-rs/slab) |
| smallvec | 1.16.5 | MIT OR Apache-2.0 | [upstream](https://github.com/servo/rust-smallvec) |
| socket2 | 0.6.5 | MIT OR Apache-2.0 | [upstream](https://github.com/rust-lang/socket2) |
| sqlcipher-src | 420.0.0+sqlcipher-4.20.0-sqlite-3.54.1-libtomcrypt-1.18.2 | MIT AND BSD-3-Clause AND blessing AND WTFPL | [upstream](https://github.com/LucaCappelletti94/sqlcipher-src) |
| sqlite-wasm-rs | 0.6.3 | MIT | [upstream](https://github.com/Spxg/sqlite-wasm-rs) |
| stable_deref_trait | 1.2.1 | MIT OR Apache-2.0 | [upstream](https://github.com/storyyeller/stable_deref_trait) |
| strsim | 0.11.1 | MIT | [upstream](https://github.com/rapidfuzz/strsim-rs) |
| subtle | 2.6.1 | BSD-3-Clause | [upstream](https://github.com/dalek-cryptography/subtle) |
| syn | 2.0.119 | MIT OR Apache-2.0 | [upstream](https://github.com/dtolnay/syn) |
| syn | 3.0.10 | MIT OR Apache-2.0 | [upstream](https://github.com/dtolnay/syn) |
| sync_wrapper | 1.0.2 | Apache-2.0 | [upstream](https://github.com/Actyx/sync_wrapper) |
| synstructure | 0.14.0 | MIT | [upstream](https://github.com/mystor/synstructure) |
| tantivy | 0.27.0 | MIT | [upstream](https://github.com/quickwit-oss/tantivy) |
| tantivy-bitpacker | 0.27.0 | MIT | [upstream](https://github.com/quickwit-oss/tantivy) |
| tantivy-columnar | 0.27.0 | MIT | [upstream](https://github.com/quickwit-oss/tantivy) |
| tantivy-common | 0.27.0 | MIT | [upstream](https://github.com/quickwit-oss/tantivy) |
| tantivy-fst | 0.5.0 | Unlicense/MIT | [upstream](https://github.com/quickwit-inc/fst) |
| tantivy-query-grammar | 0.27.0 | MIT | [upstream](https://github.com/quickwit-oss/tantivy) |
| tantivy-sstable | 0.27.0 | MIT | [upstream](https://github.com/quickwit-oss/tantivy) |
| tantivy-stacker | 0.27.0 | MIT | [upstream](https://github.com/quickwit-oss/tantivy) |
| tantivy-tokenizer-api | 0.7.0 | MIT | [upstream](https://github.com/quickwit-oss/tantivy) |
| tempfile | 3.23.0 | MIT OR Apache-2.0 | [upstream](https://github.com/Stebalien/tempfile) |
| thiserror | 2.0.21 | MIT OR Apache-2.0 | [upstream](https://github.com/dtolnay/thiserror) |
| thiserror-impl | 2.0.21 | MIT OR Apache-2.0 | [upstream](https://github.com/dtolnay/thiserror) |
| time | 0.3.55 | MIT OR Apache-2.0 | [upstream](https://github.com/time-rs/time) |
| time-core | 0.1.9 | MIT OR Apache-2.0 | [upstream](https://github.com/time-rs/time) |
| time-macros | 0.2.32 | MIT OR Apache-2.0 | [upstream](https://github.com/time-rs/time) |
| tinystr | 0.8.5 | Unicode-3.0 | [upstream](https://github.com/unicode-org/icu4x) |
| tinyvec | 1.13.3 | Zlib OR Apache-2.0 OR MIT | [upstream](https://github.com/Lokathor/tinyvec) |
| tokio | 1.54.1 | MIT | [upstream](https://github.com/tokio-rs/tokio) |
| tokio-rustls | 0.26.6 | MIT OR Apache-2.0 | [upstream](https://github.com/rustls/tokio-rustls) |
| toml | 0.8.23 | MIT OR Apache-2.0 | [upstream](https://github.com/toml-rs/toml) |
| toml_datetime | 0.6.11 | MIT OR Apache-2.0 | [upstream](https://github.com/toml-rs/toml) |
| toml_edit | 0.22.27 | MIT OR Apache-2.0 | [upstream](https://github.com/toml-rs/toml) |
| toml_write | 0.1.2 | MIT OR Apache-2.0 | [upstream](https://github.com/toml-rs/toml) |
| tower | 0.5.3 | MIT | [upstream](https://github.com/tower-rs/tower) |
| tower-http | 0.6.11 | MIT | [upstream](https://github.com/tower-rs/tower-http) |
| tower-layer | 0.3.3 | MIT | [upstream](https://github.com/tower-rs/tower) |
| tower-service | 0.3.3 | MIT | [upstream](https://github.com/tower-rs/tower) |
| tracing | 0.1.44 | MIT | [upstream](https://github.com/tokio-rs/tracing) |
| tracing-core | 0.1.36 | MIT | [upstream](https://github.com/tokio-rs/tracing) |
| try-lock | 0.2.5 | MIT | [upstream](https://github.com/seanmonstar/try-lock) |
| typeid | 1.0.3 | MIT OR Apache-2.0 | [upstream](https://github.com/dtolnay/typeid) |
| typenum | 1.20.1 | MIT OR Apache-2.0 | [upstream](https://github.com/paholg/typenum) |
| typetag | 0.2.23 | MIT OR Apache-2.0 | [upstream](https://github.com/dtolnay/typetag) |
| typetag-impl | 0.2.23 | MIT OR Apache-2.0 | [upstream](https://github.com/dtolnay/typetag) |
| unicode-ident | 1.0.27 | (MIT OR Apache-2.0) AND Unicode-3.0 | [upstream](https://github.com/dtolnay/unicode-ident) |
| unicode-normalization | 0.1.26 | (MIT OR Apache-2.0) AND Unicode-3.0 | [upstream](https://github.com/unicode-rs/unicode-normalization) |
| untrusted | 0.9.0 | ISC | [upstream](https://github.com/briansmith/untrusted) |
| unwrap-infallible | 1.0.0 | MIT OR Apache-2.0 | [upstream](https://github.com/mzabaluev/unwrap-infallible) |
| url | 2.5.8 | MIT OR Apache-2.0 | [upstream](https://github.com/servo/rust-url) |
| utf8-ranges | 1.0.5 | Unlicense/MIT | [upstream](https://github.com/BurntSushi/utf8-ranges) |
| utf8_iter | 1.1.0 | Apache-2.0 OR MIT | [upstream](https://github.com/hsivonen/utf8_iter) |
| utf_types | 1.0.0 | Apache-2.0 OR MIT | [upstream](https://github.com/hsivonen/utf_types) |
| uuid | 1.29.2 | Apache-2.0 OR MIT | [upstream](https://github.com/uuid-rs/uuid) |
| vcpkg | 0.2.15 | MIT/Apache-2.0 | [upstream](https://github.com/mcgoo/vcpkg-rs) |
| version_check | 0.9.5 | MIT/Apache-2.0 | [upstream](https://github.com/SergioBenitez/version_check) |
| want | 0.3.2 | MIT | [upstream](https://github.com/seanmonstar/want) |
| wasi | 0.11.1+wasi-snapshot-preview1 | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT | [upstream](https://github.com/bytecodealliance/wasi) |
| wasip2 | 1.0.4+wasi-0.2.12 | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT | [upstream](https://github.com/bytecodealliance/wasi-rs) |
| wasm-bindgen | 0.2.130 | MIT OR Apache-2.0 | [upstream](https://github.com/wasm-bindgen/wasm-bindgen) |
| wasm-bindgen-futures | 0.4.80 | MIT OR Apache-2.0 | [upstream](https://github.com/wasm-bindgen/wasm-bindgen/tree/master/crates/futures) |
| wasm-bindgen-macro | 0.2.130 | MIT OR Apache-2.0 | [upstream](https://github.com/wasm-bindgen/wasm-bindgen/tree/master/crates/macro) |
| wasm-bindgen-macro-support | 0.2.130 | MIT OR Apache-2.0 | [upstream](https://github.com/wasm-bindgen/wasm-bindgen/tree/main/crates/macro-support) |
| wasm-bindgen-shared | 0.2.130 | MIT OR Apache-2.0 | [upstream](https://github.com/wasm-bindgen/wasm-bindgen/tree/master/crates/shared) |
| web-sys | 0.3.107 | MIT OR Apache-2.0 | [upstream](https://github.com/wasm-bindgen/wasm-bindgen/tree/master/crates/web-sys) |
| web-time | 1.1.0 | MIT OR Apache-2.0 | [upstream](https://github.com/daxpedda/web-time) |
| webpki-roots | 1.0.12 | CDLA-Permissive-2.0 | [upstream](https://github.com/rustls/webpki-roots) |
| winapi | 0.3.9 | MIT/Apache-2.0 | [upstream](https://github.com/retep998/winapi-rs) |
| winapi-i686-pc-windows-gnu | 0.4.0 | MIT/Apache-2.0 | [upstream](https://github.com/retep998/winapi-rs) |
| winapi-x86_64-pc-windows-gnu | 0.4.0 | MIT/Apache-2.0 | [upstream](https://github.com/retep998/winapi-rs) |
| windows-link | 0.2.1 | MIT OR Apache-2.0 | [upstream](https://github.com/microsoft/windows-rs) |
| windows-sys | 0.52.0 | MIT OR Apache-2.0 | [upstream](https://github.com/microsoft/windows-rs) |
| windows-sys | 0.59.0 | MIT OR Apache-2.0 | [upstream](https://github.com/microsoft/windows-rs) |
| windows-sys | 0.61.2 | MIT OR Apache-2.0 | [upstream](https://github.com/microsoft/windows-rs) |
| windows-targets | 0.52.6 | MIT OR Apache-2.0 | [upstream](https://github.com/microsoft/windows-rs) |
| windows_aarch64_gnullvm | 0.52.6 | MIT OR Apache-2.0 | [upstream](https://github.com/microsoft/windows-rs) |
| windows_aarch64_msvc | 0.52.6 | MIT OR Apache-2.0 | [upstream](https://github.com/microsoft/windows-rs) |
| windows_i686_gnu | 0.52.6 | MIT OR Apache-2.0 | [upstream](https://github.com/microsoft/windows-rs) |
| windows_i686_gnullvm | 0.52.6 | MIT OR Apache-2.0 | [upstream](https://github.com/microsoft/windows-rs) |
| windows_i686_msvc | 0.52.6 | MIT OR Apache-2.0 | [upstream](https://github.com/microsoft/windows-rs) |
| windows_x86_64_gnu | 0.52.6 | MIT OR Apache-2.0 | [upstream](https://github.com/microsoft/windows-rs) |
| windows_x86_64_gnullvm | 0.52.6 | MIT OR Apache-2.0 | [upstream](https://github.com/microsoft/windows-rs) |
| windows_x86_64_msvc | 0.52.6 | MIT OR Apache-2.0 | [upstream](https://github.com/microsoft/windows-rs) |
| winnow | 0.7.15 | MIT | [upstream](https://github.com/winnow-rs/winnow) |
| wit-bindgen | 0.57.1 | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT | [upstream](https://github.com/bytecodealliance/wit-bindgen) |
| writeable | 0.6.4 | Unicode-3.0 | [upstream](https://github.com/unicode-org/icu4x) |
| xz2 | 0.1.7 | MIT/Apache-2.0 | [upstream](https://github.com/alexcrichton/xz2-rs) |
| yoke | 0.8.4 | Unicode-3.0 | [upstream](https://github.com/unicode-org/icu4x) |
| yoke-derive | 0.8.4 | Unicode-3.0 | [upstream](https://github.com/unicode-org/icu4x) |
| zerocopy | 0.8.63 | BSD-2-Clause OR Apache-2.0 OR MIT | [upstream](https://github.com/google/zerocopy) |
| zerocopy-derive | 0.8.63 | BSD-2-Clause OR Apache-2.0 OR MIT | [upstream](https://github.com/google/zerocopy) |
| zerofrom | 0.1.8 | Unicode-3.0 | [upstream](https://github.com/unicode-org/icu4x) |
| zerofrom-derive | 0.1.8 | Unicode-3.0 | [upstream](https://github.com/unicode-org/icu4x) |
| zeroize | 1.9.1 | Apache-2.0 OR MIT | [upstream](https://github.com/RustCrypto/utils) |
| zerotrie | 0.2.5 | Unicode-3.0 | [upstream](https://github.com/unicode-org/icu4x) |
| zerovec | 0.11.9 | Unicode-3.0 | [upstream](https://github.com/unicode-org/icu4x) |
| zerovec-derive | 0.11.7 | Unicode-3.0 | [upstream](https://github.com/unicode-org/icu4x) |
| zmij | 1.0.23 | MIT | [upstream](https://github.com/dtolnay/zmij) |
| zstd | 0.14.3 | BSD-3-Clause | [upstream](https://github.com/gyscos/zstd-rs) |
| zstd-safe | 8.2.0 | BSD-3-Clause | [upstream](https://github.com/gyscos/zstd-rs) |
| zstd-sys | 2.1.2+zstd.1.5.7 | BSD-3-Clause | [upstream](https://github.com/gyscos/zstd-rs) |

## Added in Task 4

Core now directly pins html5ever 0.39.0 for its streaming HTML5 tokenizer,
rayon 1.12.0 for bounded extraction workers, serde 1.0.228 and serde_json 1.0.154
for versioned index metadata. Existing direct Tantivy remains exactly 0.27.0.
Rayon/Serde/JSON were already resolved and recorded above. The additional exact
registry versions below were checked using locked offline Cargo metadata on
2026-11-11 against the downloaded package manifests. These are upstream license
declarations, not a new compatibility or full-license audit. No native library
was added by the HTML tokenizer.

| Dependency | Exact version | Declared license | Source |
| --- | --- | --- | --- |
| html5ever | 0.39.0 | MIT OR Apache-2.0 | [upstream](https://github.com/servo/html5ever) |
| lock_api | 0.4.14 | MIT OR Apache-2.0 | [upstream](https://github.com/Amanieu/parking_lot) |
| markup5ever | 0.39.0 | MIT OR Apache-2.0 | [upstream](https://github.com/servo/html5ever) |
| new_debug_unreachable | 1.0.6 | MIT | [upstream](https://github.com/mbrubeck/rust-debug-unreachable) |
| parking_lot | 0.12.5 | MIT OR Apache-2.0 | [upstream](https://github.com/Amanieu/parking_lot) |
| parking_lot_core | 0.9.12 | MIT OR Apache-2.0 | [upstream](https://github.com/Amanieu/parking_lot) |
| phf | 0.13.1 | MIT | [upstream](https://github.com/rust-phf/rust-phf) |
| phf_codegen | 0.13.1 | MIT | [upstream](https://github.com/rust-phf/rust-phf) |
| phf_generator | 0.13.1 | MIT | [upstream](https://github.com/rust-phf/rust-phf) |
| phf_shared | 0.13.1 | MIT | [upstream](https://github.com/rust-phf/rust-phf) |
| precomputed-hash | 0.1.1 | MIT | [upstream](https://github.com/emilio/precomputed-hash) |
| redox_syscall | 0.5.18 | MIT | [upstream](https://gitlab.redox-os.org/redox-os/syscall) |
| scopeguard | 1.2.0 | MIT OR Apache-2.0 | [upstream](https://github.com/bluss/scopeguard) |
| siphasher | 1.0.4 | MIT OR Apache-2.0 | [upstream](https://github.com/jedisct1/rust-siphash) |
| string_cache | 0.9.0 | MIT OR Apache-2.0 | [upstream](https://github.com/servo/string-cache) |
| string_cache_codegen | 0.6.1 | MIT OR Apache-2.0 | [upstream](https://github.com/servo/string-cache) |
| tendril | 0.5.1 | MIT OR Apache-2.0 | [upstream](https://github.com/servo/html5ever) |
| web_atoms | 0.2.6 | MIT OR Apache-2.0 | [upstream](https://github.com/servo/html5ever) |

## Task 5 encrypted notes dependencies

Pinned runtime additions are Argon2 0.5.3 (`alloc`, `zeroize`, defaults disabled),
chacha20poly1305 0.11.0 (`alloc`, `zeroize`, defaults disabled), getrandom 0.4.3,
subtle 2.6.1 and zeroize 1.9.1. The latter three were already resolved and listed
above. Rusqlite 0.42.0 and its existing bundled SQLCipher/OpenSSL feature are
reused; there is no new FFI wrapper or alternative plaintext database backend.

The exact locked metadata was checked against downloaded manifests on
2026-11-11: 300 registry packages. The new registry entries below supplement the
earlier tables. These are upstream license declarations, not a new compatibility
or security audit. Argon2 uses its fixed v19 Argon2id profile; AEAD uses the
XChaCha20Poly1305 implementation from RustCrypto. Protocol and build evidence
is in [notes-validation.md](docs/spike/notes-validation.md).

| Dependency | Exact version | Declared license | Source |
| --- | --- | --- | --- |
| aead | 0.6.1 | MIT OR Apache-2.0 | [upstream](https://github.com/RustCrypto/traits) |
| argon2 | 0.5.3 | MIT OR Apache-2.0 | [upstream](https://github.com/RustCrypto/password-hashes/tree/master/argon2) |
| base64ct | 1.8.3 | Apache-2.0 OR MIT | [upstream](https://github.com/RustCrypto/formats) |
| blake2 | 0.10.6 | MIT OR Apache-2.0 | [upstream](https://github.com/RustCrypto/hashes) |
| chacha20poly1305 | 0.11.0 | Apache-2.0 OR MIT | [upstream](https://github.com/RustCrypto/AEADs) |
| cipher | 0.5.2 | MIT OR Apache-2.0 | [upstream](https://github.com/RustCrypto/traits) |
| cmov | 0.5.4 | Apache-2.0 OR MIT | [upstream](https://github.com/RustCrypto/utils) |
| ctutils | 0.4.8 | Apache-2.0 OR MIT | [upstream](https://github.com/RustCrypto/utils) |
| inout | 0.2.6 | MIT OR Apache-2.0 | [upstream](https://github.com/RustCrypto/utils) |
| password-hash | 0.5.0 | MIT OR Apache-2.0 | [upstream](https://github.com/RustCrypto/traits/tree/master/password-hash) |
| poly1305 | 0.9.1 | Apache-2.0 OR MIT | [upstream](https://github.com/RustCrypto/universal-hashes) |
| rand_core | 0.6.4 | MIT OR Apache-2.0 | [upstream](https://github.com/rust-random/rand) |
| universal-hash | 0.6.1 | MIT OR Apache-2.0 | [upstream](https://github.com/RustCrypto/traits) |

## Added in Task 6 Step 0

The loopback host directly pins axum **0.8.9**, percent-encoding **2.3.2**,
Tokio **1.54.1** and tokio-util **0.7.20**. The latter two provide the owned
runtime and bounded file streaming. The CLI directly pins ctrlc **3.5.2** with
its termination feature. Core HTTP tests explicitly depend on reqwest
**0.12.24** with blocking/JSON support and default features disabled.
Existing exact declarations for percent-encoding, Tokio and reqwest remain in
the earlier table. The additional locked registry packages below were checked
with `cargo metadata --format-version 1 --locked --offline` on 2026-11-11.
Some are target-specific resolved packages; this is declaration/provenance
evidence, not a compatibility audit or a build claim for every target.

| Dependency | Exact version | Declared license | Source |
| --- | --- | --- | --- |
| axum | 0.8.9 | MIT | [upstream](https://github.com/tokio-rs/axum) |
| axum-core | 0.5.6 | MIT | [upstream](https://github.com/tokio-rs/axum) |
| block2 | 0.6.2 | MIT | [upstream](https://github.com/madsmtm/objc2) |
| ctrlc | 3.5.2 | MIT/Apache-2.0 | [upstream](https://github.com/Detegr/rust-ctrlc) |
| dispatch2 | 0.3.1 | Zlib OR Apache-2.0 OR MIT | [upstream](https://github.com/madsmtm/objc2) |
| httpdate | 1.0.3 | MIT OR Apache-2.0 | [upstream](https://github.com/pyfisch/httpdate) |
| matchit | 0.8.4 | MIT AND BSD-3-Clause | [upstream](https://github.com/ibraheemdev/matchit) |
| mime | 0.3.18 | MIT OR Apache-2.0 | [upstream](https://github.com/hyperium/mime) |
| nix | 0.31.3 | MIT | [upstream](https://github.com/nix-rust/nix) |
| objc2 | 0.6.5 | MIT | [upstream](https://github.com/madsmtm/objc2) |
| objc2-encode | 4.1.0 | MIT | [upstream](https://github.com/madsmtm/objc2) |
| serde_path_to_error | 0.1.20 | MIT OR Apache-2.0 | [upstream](https://github.com/dtolnay/path-to-error) |
| tokio-macros | 2.7.2 | MIT | [upstream](https://github.com/tokio-rs/tokio) |
| tokio-util | 0.7.20 | MIT | [upstream](https://github.com/tokio-rs/tokio) |

## Added in Task 6 Step 2

Core directly promotes the already-resolved futures-util **0.3.34** (std,
defaults disabled) for bounded asynchronous range streams. Its declaration
remains in the earlier table. Core now uses the reader-only vendored pmtiles
**0.28.0** fork, upstream revision `dcd4051954792ff0d1a9390cc15a5381ca873a72`,
declaring **MIT OR Apache-2.0**. Original license files are retained. Official
archive SHA-256 is
`7a242b695fabc0870d663c3883e78ead4f888134c0e83fbf00fcc35d60005015`;
see [fork changes and source provenance](crates/pmtiles-fork/TONALLI.md).
The fork exposes no network, mmap, writer or unbounded cache features.

The following additions were checked from exact downloaded manifests using
locked offline Cargo metadata on 2026-11-11, after fetching missing test
dependencies. This includes development-only dependencies of the fork.
Declarations are recorded without claiming a complete compatibility audit.

| Dependency | Exact version | Declared license | Source |
| --- | --- | --- | --- |
| fast_hilbert | 2.1.0 | MIT | [upstream](https://github.com/becheran/fast-hilbert) |
| flate2 | 1.1.10 | MIT OR Apache-2.0 | [upstream](https://github.com/rust-lang/flate2-rs) |
| futures-timer | 3.0.4 | MIT/Apache-2.0 | [upstream](https://github.com/async-rs/futures-timer) |
| glob | 0.3.4 | MIT OR Apache-2.0 | [upstream](https://github.com/rust-lang/glob) |
| integer-encoding | 4.1.0 | MIT | [upstream](https://github.com/dermesser/integer-encoding-rs) |
| proc-macro-crate | 3.5.0 | MIT OR Apache-2.0 | [upstream](https://github.com/bkchr/proc-macro-crate) |
| relative-path | 1.9.3 | MIT OR Apache-2.0 | [upstream](https://github.com/udoprog/relative-path) |
| rstest | 0.28.0 | MIT OR Apache-2.0 | [upstream](https://github.com/la10736/rstest) |
| rstest_macros | 0.28.0 | MIT OR Apache-2.0 | [upstream](https://github.com/la10736/rstest) |
| rustc_version | 0.4.1 | MIT OR Apache-2.0 | [upstream](https://github.com/djc/rustc-version-rs) |
| semver | 1.0.28 | MIT OR Apache-2.0 | [upstream](https://github.com/dtolnay/semver) |
| toml_datetime | 1.1.3+spec-1.1.0 | MIT OR Apache-2.0 | [upstream](https://github.com/toml-rs/toml) |
| toml_edit | 0.25.19+spec-1.1.0 | MIT OR Apache-2.0 | [upstream](https://github.com/toml-rs/toml) |
| toml_parser | 1.1.5+spec-1.1.0 | MIT OR Apache-2.0 | [upstream](https://github.com/toml-rs/toml) |
| winnow | 1.0.5 | MIT | [upstream](https://github.com/winnow-rs/winnow) |
| zlib-rs | 0.6.8 | Zlib | [upstream](https://github.com/trifectatechfoundation/zlib-rs) |

## Vendored ZIM reader

`crates/zim-fork` is a modified `zim` 0.5.0, upstream commit
`82b897e479254601efdabc0f37c1cb9e7e20f7d3` from
https://github.com/dignifiedquire/zim. Official crate archive SHA-256
`63cffa116b0aaebeb94142acd0ad39c63ee9f6180a5729e5c02036b73b4b082e`
matched crates.io metadata. The original Apache-2.0 and MIT texts are retained;
the fork declares Apache-2.0 OR MIT. Modified-file notices and removed CLI
dependencies are recorded in [FORK.md](crates/zim-fork/FORK.md).
Its direct dependencies are bitreader 0.3.11, byteorder 1.5.0, md-5 0.11.0,
thiserror 2.0.21, static xz2 0.1.7 and zstd 0.14.3. Upstream path-open checks,
core fixture tests and the Android ARM64/API 34 release rebuild passed; these
are not phone execution or full-Wikipedia performance evidence.

## Native libraries selected for the core smoke build

The native versions below were checked in the locked crate sources on
2026-11-11. The Rust crate license declarations above do not replace these
native-source licenses. This records provenance and declarations rather than
a legal compatibility audit.

| Native component | Exact version | Source/license evidence |
| --- | --- | --- |
| SQLCipher | 4.20.0, SQLite 3.54.1 amalgamation | `sqlcipher-src` 420.0.0+sqlcipher-4.20.0-sqlite-3.54.1-libtomcrypt-1.18.2; SQLCipher BSD-3-Clause, SQLite public-domain blessing. The crate declares `MIT AND BSD-3-Clause AND blessing AND WTFPL` for all shipped sources, including bundled LibTomCrypt 1.18.2. This build selects OpenSSL as the crypto provider, not LibTomCrypt. [source](https://github.com/LucaCappelletti94/sqlcipher-src) |
| OpenSSL | 3.6.3 | `openssl-src` 300.6.1+3.6.3; bundled `openssl/LICENSE.txt` declares Apache-2.0; the Rust build helper declares MIT/Apache-2.0. [source](https://github.com/openssl/openssl/tree/openssl-3.6.3) |
| Zstandard | 1.5.7 | `zstd-sys` 2.1.2+zstd.1.5.7; native `zstd/LICENSE` declares BSD-3-Clause, with GPL-2.0 available as an alternative in `zstd/COPYING`. [source](https://github.com/facebook/zstd/tree/v1.5.7) |
| XZ liblzma | 5.2.5 | `lzma-sys` 0.1.20; `xz-5.2/src/liblzma/api/lzma/version.h` identifies 5.2.5 and `xz-5.2/COPYING` places liblzma in the public domain. The Rust bindings declare MIT/Apache-2.0. [source](https://github.com/alexcrichton/xz2-rs) |

## Development tool checked separately

`cargo-ndk` **4.1.2** is an installed development build tool, not a shipped core
dependency. Its cached exact crate manifest declares **Apache-2.0 OR MIT** and
the upstream is [bbqsrc/cargo-ndk](https://github.com/bbqsrc/cargo-ndk). It drove
the recorded Android ARM64/API 34 native builds; see
[native-gate-validation.md](docs/spike/native-gate-validation.md). This note
records that tool's declaration, not a new audit of its separately installed
dependency tree.

## Web UI dependencies

Checked 2026-11-11 against npm 10.9.8 package-lock.json. Exact versions are pinned;
licenses below are upstream declarations, not a security/compatibility audit.
MapLibre 4.7.1, PMTiles 4.4.0 and Protomaps basemaps 5.7.0 declare BSD-3-Clause.
Playwright Core 1.65.0 is an Apache-2.0 development-only browser verifier; it
uses installed Microsoft Edge and does not download or ship a bundled browser.
Optional lock entries do not prove Android execution.

Vitest 5.1.0 is a pinned MIT development-only runner for Node fake-timer tests.
Its Node engine is `^22.12.0 || ^24.0.0 || >=26.0.0`, satisfied by Node 22.23.2;
its Vite peer range includes the pinned Vite 8.3.8. No DOM/browser adapter was added.

React UI pins: React/React DOM 19.3.1, React Router DOM 7.18.5, TanStack
React Query 5.107.1, TanStack React Virtual 3.14.26, Tailwind CSS and its Vite
plugin 4.3.5 (all declare MIT). Installed with scripts/audit/funding disabled.

| Dependency | Exact version | Declared license | Source |
| --- | --- | --- | --- |
| @emnapi/core | 2.0.0-beta.2 | MIT | [registry archive](https://registry.npmjs.org/@emnapi/core/-/core-2.0.0-beta.2.tgz) |
| @emnapi/runtime | 2.0.0-beta.2 | MIT | [registry archive](https://registry.npmjs.org/@emnapi/runtime/-/runtime-2.0.0-beta.2.tgz) |
| @emnapi/wasi-threads | 2.2.0 | MIT | [registry archive](https://registry.npmjs.org/@emnapi/wasi-threads/-/wasi-threads-2.2.0.tgz) |
| @jridgewell/gen-mapping | 0.3.13 | MIT | [registry archive](https://registry.npmjs.org/@jridgewell/gen-mapping/-/gen-mapping-0.3.13.tgz) |
| @jridgewell/remapping | 2.3.5 | MIT | [registry archive](https://registry.npmjs.org/@jridgewell/remapping/-/remapping-2.3.5.tgz) |
| @jridgewell/resolve-uri | 3.1.2 | MIT | [registry archive](https://registry.npmjs.org/@jridgewell/resolve-uri/-/resolve-uri-3.1.2.tgz) |
| @jridgewell/sourcemap-codec | 1.6.0 | MIT | [registry archive](https://registry.npmjs.org/@jridgewell/sourcemap-codec/-/sourcemap-codec-1.6.0.tgz) |
| @jridgewell/trace-mapping | 0.3.31 | MIT | [registry archive](https://registry.npmjs.org/@jridgewell/trace-mapping/-/trace-mapping-0.3.31.tgz) |
| @mapbox/geojson-rewind | 0.5.2 | ISC | [registry archive](https://registry.npmjs.org/@mapbox/geojson-rewind/-/geojson-rewind-0.5.2.tgz) |
| @mapbox/jsonlint-lines-primitives | 2.0.3 | MIT | [registry archive](https://registry.npmjs.org/@mapbox/jsonlint-lines-primitives/-/jsonlint-lines-primitives-2.0.3.tgz) |
| @mapbox/point-geometry | 0.1.0 | ISC | [registry archive](https://registry.npmjs.org/@mapbox/point-geometry/-/point-geometry-0.1.0.tgz) |
| @mapbox/tiny-sdf | 2.3.0 | BSD-2-Clause | [registry archive](https://registry.npmjs.org/@mapbox/tiny-sdf/-/tiny-sdf-2.3.0.tgz) |
| @mapbox/unitbezier | 0.0.1 | BSD-2-Clause | [registry archive](https://registry.npmjs.org/@mapbox/unitbezier/-/unitbezier-0.0.1.tgz) |
| @mapbox/vector-tile | 1.3.1 | BSD-3-Clause | [registry archive](https://registry.npmjs.org/@mapbox/vector-tile/-/vector-tile-1.3.1.tgz) |
| @mapbox/whoots-js | 3.1.0 | ISC | [registry archive](https://registry.npmjs.org/@mapbox/whoots-js/-/whoots-js-3.1.0.tgz) |
| @maplibre/maplibre-gl-style-spec | 20.4.0 | ISC | [registry archive](https://registry.npmjs.org/@maplibre/maplibre-gl-style-spec/-/maplibre-gl-style-spec-20.4.0.tgz) |
| quickselect | 2.0.0 | ISC | [registry archive](https://registry.npmjs.org/quickselect/-/quickselect-2.0.0.tgz) |
| @napi-rs/wasm-runtime | 1.2.6 | MIT | [registry archive](https://registry.npmjs.org/@napi-rs/wasm-runtime/-/wasm-runtime-1.2.6.tgz) |
| @oxc-project/types | 0.156.0 | MIT | [registry archive](https://registry.npmjs.org/@oxc-project/types/-/types-0.156.0.tgz) |
| @protomaps/basemaps | 5.7.0 | BSD-3-Clause | [registry archive](https://registry.npmjs.org/@protomaps/basemaps/-/basemaps-5.7.0.tgz) |
| @rolldown/binding-android-arm-eabi | 1.2.19 | MIT | [registry archive](https://registry.npmjs.org/@rolldown/binding-android-arm-eabi/-/binding-android-arm-eabi-1.2.19.tgz) |
| @rolldown/binding-android-arm64 | 1.2.19 | MIT | [registry archive](https://registry.npmjs.org/@rolldown/binding-android-arm64/-/binding-android-arm64-1.2.19.tgz) |
| @rolldown/binding-darwin-arm64 | 1.2.19 | MIT | [registry archive](https://registry.npmjs.org/@rolldown/binding-darwin-arm64/-/binding-darwin-arm64-1.2.19.tgz) |
| @rolldown/binding-darwin-x64 | 1.2.19 | MIT | [registry archive](https://registry.npmjs.org/@rolldown/binding-darwin-x64/-/binding-darwin-x64-1.2.19.tgz) |
| @rolldown/binding-freebsd-x64 | 1.2.19 | MIT | [registry archive](https://registry.npmjs.org/@rolldown/binding-freebsd-x64/-/binding-freebsd-x64-1.2.19.tgz) |
| @rolldown/binding-linux-arm-gnueabihf | 1.2.19 | MIT | [registry archive](https://registry.npmjs.org/@rolldown/binding-linux-arm-gnueabihf/-/binding-linux-arm-gnueabihf-1.2.19.tgz) |
| @rolldown/binding-linux-arm64-gnu | 1.2.19 | MIT | [registry archive](https://registry.npmjs.org/@rolldown/binding-linux-arm64-gnu/-/binding-linux-arm64-gnu-1.2.19.tgz) |
| @rolldown/binding-linux-arm64-musl | 1.2.19 | MIT | [registry archive](https://registry.npmjs.org/@rolldown/binding-linux-arm64-musl/-/binding-linux-arm64-musl-1.2.19.tgz) |
| @rolldown/binding-linux-ppc64-gnu | 1.2.19 | MIT | [registry archive](https://registry.npmjs.org/@rolldown/binding-linux-ppc64-gnu/-/binding-linux-ppc64-gnu-1.2.19.tgz) |
| @rolldown/binding-linux-s390x-gnu | 1.2.19 | MIT | [registry archive](https://registry.npmjs.org/@rolldown/binding-linux-s390x-gnu/-/binding-linux-s390x-gnu-1.2.19.tgz) |
| @rolldown/binding-linux-x64-gnu | 1.2.19 | MIT | [registry archive](https://registry.npmjs.org/@rolldown/binding-linux-x64-gnu/-/binding-linux-x64-gnu-1.2.19.tgz) |
| @rolldown/binding-linux-x64-musl | 1.2.19 | MIT | [registry archive](https://registry.npmjs.org/@rolldown/binding-linux-x64-musl/-/binding-linux-x64-musl-1.2.19.tgz) |
| @rolldown/binding-openharmony-arm64 | 1.2.19 | MIT | [registry archive](https://registry.npmjs.org/@rolldown/binding-openharmony-arm64/-/binding-openharmony-arm64-1.2.19.tgz) |
| @rolldown/binding-wasm32-wasi | 1.2.19 | MIT | [registry archive](https://registry.npmjs.org/@rolldown/binding-wasm32-wasi/-/binding-wasm32-wasi-1.2.19.tgz) |
| @rolldown/binding-win32-arm64-msvc | 1.2.19 | MIT | [registry archive](https://registry.npmjs.org/@rolldown/binding-win32-arm64-msvc/-/binding-win32-arm64-msvc-1.2.19.tgz) |
| @rolldown/binding-win32-x64-msvc | 1.2.19 | MIT | [registry archive](https://registry.npmjs.org/@rolldown/binding-win32-x64-msvc/-/binding-win32-x64-msvc-1.2.19.tgz) |
| @rolldown/pluginutils | 1.1.0 | MIT | [registry archive](https://registry.npmjs.org/@rolldown/pluginutils/-/pluginutils-1.1.0.tgz) |
| @tailwindcss/node | 4.3.5 | MIT | [registry archive](https://registry.npmjs.org/@tailwindcss/node/-/node-4.3.5.tgz) |
| @tailwindcss/oxide | 4.3.5 | MIT | [registry archive](https://registry.npmjs.org/@tailwindcss/oxide/-/oxide-4.3.5.tgz) |
| @tailwindcss/oxide-android-arm64 | 4.3.5 | MIT | [registry archive](https://registry.npmjs.org/@tailwindcss/oxide-android-arm64/-/oxide-android-arm64-4.3.5.tgz) |
| @tailwindcss/oxide-darwin-arm64 | 4.3.5 | MIT | [registry archive](https://registry.npmjs.org/@tailwindcss/oxide-darwin-arm64/-/oxide-darwin-arm64-4.3.5.tgz) |
| @tailwindcss/oxide-darwin-x64 | 4.3.5 | MIT | [registry archive](https://registry.npmjs.org/@tailwindcss/oxide-darwin-x64/-/oxide-darwin-x64-4.3.5.tgz) |
| @tailwindcss/oxide-freebsd-x64 | 4.3.5 | MIT | [registry archive](https://registry.npmjs.org/@tailwindcss/oxide-freebsd-x64/-/oxide-freebsd-x64-4.3.5.tgz) |
| @tailwindcss/oxide-linux-arm-gnueabihf | 4.3.5 | MIT | [registry archive](https://registry.npmjs.org/@tailwindcss/oxide-linux-arm-gnueabihf/-/oxide-linux-arm-gnueabihf-4.3.5.tgz) |
| @tailwindcss/oxide-linux-arm64-gnu | 4.3.5 | MIT | [registry archive](https://registry.npmjs.org/@tailwindcss/oxide-linux-arm64-gnu/-/oxide-linux-arm64-gnu-4.3.5.tgz) |
| @tailwindcss/oxide-linux-arm64-musl | 4.3.5 | MIT | [registry archive](https://registry.npmjs.org/@tailwindcss/oxide-linux-arm64-musl/-/oxide-linux-arm64-musl-4.3.5.tgz) |
| @tailwindcss/oxide-linux-x64-gnu | 4.3.5 | MIT | [registry archive](https://registry.npmjs.org/@tailwindcss/oxide-linux-x64-gnu/-/oxide-linux-x64-gnu-4.3.5.tgz) |
| @tailwindcss/oxide-linux-x64-musl | 4.3.5 | MIT | [registry archive](https://registry.npmjs.org/@tailwindcss/oxide-linux-x64-musl/-/oxide-linux-x64-musl-4.3.5.tgz) |
| @tailwindcss/oxide-wasm32-wasi | 4.3.5 | MIT | [registry archive](https://registry.npmjs.org/@tailwindcss/oxide-wasm32-wasi/-/oxide-wasm32-wasi-4.3.5.tgz) |
| @emnapi/core | 1.11.3 | MIT | [bundled archive](https://registry.npmjs.org/@tailwindcss/oxide-wasm32-wasi/-/oxide-wasm32-wasi-4.3.5.tgz) |
| @emnapi/runtime | 1.11.3 | MIT | [bundled archive](https://registry.npmjs.org/@tailwindcss/oxide-wasm32-wasi/-/oxide-wasm32-wasi-4.3.5.tgz) |
| @emnapi/wasi-threads | 1.2.3 | MIT | [bundled archive](https://registry.npmjs.org/@tailwindcss/oxide-wasm32-wasi/-/oxide-wasm32-wasi-4.3.5.tgz) |
| @napi-rs/wasm-runtime | 1.2.2 | MIT | [bundled archive](https://registry.npmjs.org/@tailwindcss/oxide-wasm32-wasi/-/oxide-wasm32-wasi-4.3.5.tgz) |
| @tybys/wasm-util | 0.10.3 | MIT | [bundled archive](https://registry.npmjs.org/@tailwindcss/oxide-wasm32-wasi/-/oxide-wasm32-wasi-4.3.5.tgz) |
| tslib | 2.8.1 | 0BSD | [bundled archive](https://registry.npmjs.org/@tailwindcss/oxide-wasm32-wasi/-/oxide-wasm32-wasi-4.3.5.tgz) |
| @tailwindcss/oxide-win32-arm64-msvc | 4.3.5 | MIT | [registry archive](https://registry.npmjs.org/@tailwindcss/oxide-win32-arm64-msvc/-/oxide-win32-arm64-msvc-4.3.5.tgz) |
| @tailwindcss/oxide-win32-x64-msvc | 4.3.5 | MIT | [registry archive](https://registry.npmjs.org/@tailwindcss/oxide-win32-x64-msvc/-/oxide-win32-x64-msvc-4.3.5.tgz) |
| @tailwindcss/vite | 4.3.5 | MIT | [registry archive](https://registry.npmjs.org/@tailwindcss/vite/-/vite-4.3.5.tgz) |
| @tanstack/query-core | 5.107.1 | MIT | [registry archive](https://registry.npmjs.org/@tanstack/query-core/-/query-core-5.107.1.tgz) |
| @tanstack/react-query | 5.107.1 | MIT | [registry archive](https://registry.npmjs.org/@tanstack/react-query/-/react-query-5.107.1.tgz) |
| @tanstack/react-virtual | 3.14.26 | MIT | [registry archive](https://registry.npmjs.org/@tanstack/react-virtual/-/react-virtual-3.14.26.tgz) |
| @tanstack/virtual-core | 3.19.11 | MIT | [registry archive](https://registry.npmjs.org/@tanstack/virtual-core/-/virtual-core-3.19.11.tgz) |
| @tybys/wasm-util | 0.10.4 | MIT | [registry archive](https://registry.npmjs.org/@tybys/wasm-util/-/wasm-util-0.10.4.tgz) |
| @types/chai | 5.2.3 | MIT | [registry archive](https://registry.npmjs.org/@types/chai/-/chai-5.2.3.tgz) |
| @types/deep-eql | 4.0.2 | MIT | [registry archive](https://registry.npmjs.org/@types/deep-eql/-/deep-eql-4.0.2.tgz) |
| @types/estree | 1.0.10 | MIT | [registry archive](https://registry.npmjs.org/@types/estree/-/estree-1.0.10.tgz) |
| @types/geojson | 7946.0.16 | MIT | [registry archive](https://registry.npmjs.org/@types/geojson/-/geojson-7946.0.16.tgz) |
| @types/geojson-vt | 3.2.5 | MIT | [registry archive](https://registry.npmjs.org/@types/geojson-vt/-/geojson-vt-3.2.5.tgz) |
| @types/mapbox__point-geometry | 0.1.4 | MIT | [registry archive](https://registry.npmjs.org/@types/mapbox__point-geometry/-/mapbox__point-geometry-0.1.4.tgz) |
| @types/mapbox__vector-tile | 1.3.4 | MIT | [registry archive](https://registry.npmjs.org/@types/mapbox__vector-tile/-/mapbox__vector-tile-1.3.4.tgz) |
| @types/pbf | 3.0.5 | MIT | [registry archive](https://registry.npmjs.org/@types/pbf/-/pbf-3.0.5.tgz) |
| @types/supercluster | 7.1.3 | MIT | [registry archive](https://registry.npmjs.org/@types/supercluster/-/supercluster-7.1.3.tgz) |
| @vitest/mocker | 5.1.0 | MIT | [registry archive](https://registry.npmjs.org/@vitest/mocker/-/mocker-5.1.0.tgz) |
| @vitest/spy | 5.1.0 | MIT | [registry archive](https://registry.npmjs.org/@vitest/spy/-/spy-5.1.0.tgz) |
| assertion-error | 2.0.1 | MIT | [registry archive](https://registry.npmjs.org/assertion-error/-/assertion-error-2.0.1.tgz) |
| chai | 6.3.0 | MIT | [registry archive](https://registry.npmjs.org/chai/-/chai-6.3.0.tgz) |
| cookie | 1.1.1 | MIT | [registry archive](https://registry.npmjs.org/cookie/-/cookie-1.1.1.tgz) |
| detect-libc | 2.1.2 | Apache-2.0 | [registry archive](https://registry.npmjs.org/detect-libc/-/detect-libc-2.1.2.tgz) |
| earcut | 3.2.4 | ISC | [registry archive](https://registry.npmjs.org/earcut/-/earcut-3.2.4.tgz) |
| enhanced-resolve | 5.26.1 | MIT | [registry archive](https://registry.npmjs.org/enhanced-resolve/-/enhanced-resolve-5.26.1.tgz) |
| es-module-lexer | 2.3.2 | MIT | [registry archive](https://registry.npmjs.org/es-module-lexer/-/es-module-lexer-2.3.2.tgz) |
| estree-walker | 3.0.3 | MIT | [registry archive](https://registry.npmjs.org/estree-walker/-/estree-walker-3.0.3.tgz) |
| expect-type | 1.4.0 | Apache-2.0 | [registry archive](https://registry.npmjs.org/expect-type/-/expect-type-1.4.0.tgz) |
| fdir | 6.5.0 | MIT | [registry archive](https://registry.npmjs.org/fdir/-/fdir-6.5.0.tgz) |
| fflate | 0.8.3 | MIT | [registry archive](https://registry.npmjs.org/fflate/-/fflate-0.8.3.tgz) |
| fsevents | 2.3.3 | MIT | [registry archive](https://registry.npmjs.org/fsevents/-/fsevents-2.3.3.tgz) |
| geojson-vt | 4.0.3 | ISC | [registry archive](https://registry.npmjs.org/geojson-vt/-/geojson-vt-4.0.3.tgz) |
| get-stream | 6.0.1 | MIT | [registry archive](https://registry.npmjs.org/get-stream/-/get-stream-6.0.1.tgz) |
| gl-matrix | 3.4.4 | MIT | [registry archive](https://registry.npmjs.org/gl-matrix/-/gl-matrix-3.4.4.tgz) |
| global-prefix | 4.0.0 | MIT | [registry archive](https://registry.npmjs.org/global-prefix/-/global-prefix-4.0.0.tgz) |
| graceful-fs | 4.2.11 | ISC | [registry archive](https://registry.npmjs.org/graceful-fs/-/graceful-fs-4.2.11.tgz) |
| ieee754 | 1.2.1 | BSD-3-Clause | [registry archive](https://registry.npmjs.org/ieee754/-/ieee754-1.2.1.tgz) |
| ini | 4.1.3 | ISC | [registry archive](https://registry.npmjs.org/ini/-/ini-4.1.3.tgz) |
| isexe | 3.1.5 | BlueOak-1.0.0 | [registry archive](https://registry.npmjs.org/isexe/-/isexe-3.1.5.tgz) |
| jiti | 2.7.0 | MIT | [registry archive](https://registry.npmjs.org/jiti/-/jiti-2.7.0.tgz) |
| json-stringify-pretty-compact | 4.0.0 | MIT | [registry archive](https://registry.npmjs.org/json-stringify-pretty-compact/-/json-stringify-pretty-compact-4.0.0.tgz) |
| kdbush | 4.1.0 | ISC | [registry archive](https://registry.npmjs.org/kdbush/-/kdbush-4.1.0.tgz) |
| kind-of | 6.0.3 | MIT | [registry archive](https://registry.npmjs.org/kind-of/-/kind-of-6.0.3.tgz) |
| lightningcss | 1.33.0 | MPL-2.0 | [registry archive](https://registry.npmjs.org/lightningcss/-/lightningcss-1.33.0.tgz) |
| lightningcss-android-arm64 | 1.33.0 | MPL-2.0 | [registry archive](https://registry.npmjs.org/lightningcss-android-arm64/-/lightningcss-android-arm64-1.33.0.tgz) |
| lightningcss-darwin-arm64 | 1.33.0 | MPL-2.0 | [registry archive](https://registry.npmjs.org/lightningcss-darwin-arm64/-/lightningcss-darwin-arm64-1.33.0.tgz) |
| lightningcss-darwin-x64 | 1.33.0 | MPL-2.0 | [registry archive](https://registry.npmjs.org/lightningcss-darwin-x64/-/lightningcss-darwin-x64-1.33.0.tgz) |
| lightningcss-freebsd-x64 | 1.33.0 | MPL-2.0 | [registry archive](https://registry.npmjs.org/lightningcss-freebsd-x64/-/lightningcss-freebsd-x64-1.33.0.tgz) |
| lightningcss-linux-arm-gnueabihf | 1.33.0 | MPL-2.0 | [registry archive](https://registry.npmjs.org/lightningcss-linux-arm-gnueabihf/-/lightningcss-linux-arm-gnueabihf-1.33.0.tgz) |
| lightningcss-linux-arm64-gnu | 1.33.0 | MPL-2.0 | [registry archive](https://registry.npmjs.org/lightningcss-linux-arm64-gnu/-/lightningcss-linux-arm64-gnu-1.33.0.tgz) |
| lightningcss-linux-arm64-musl | 1.33.0 | MPL-2.0 | [registry archive](https://registry.npmjs.org/lightningcss-linux-arm64-musl/-/lightningcss-linux-arm64-musl-1.33.0.tgz) |
| lightningcss-linux-x64-gnu | 1.33.0 | MPL-2.0 | [registry archive](https://registry.npmjs.org/lightningcss-linux-x64-gnu/-/lightningcss-linux-x64-gnu-1.33.0.tgz) |
| lightningcss-linux-x64-musl | 1.33.0 | MPL-2.0 | [registry archive](https://registry.npmjs.org/lightningcss-linux-x64-musl/-/lightningcss-linux-x64-musl-1.33.0.tgz) |
| lightningcss-win32-arm64-msvc | 1.33.0 | MPL-2.0 | [registry archive](https://registry.npmjs.org/lightningcss-win32-arm64-msvc/-/lightningcss-win32-arm64-msvc-1.33.0.tgz) |
| lightningcss-win32-x64-msvc | 1.33.0 | MPL-2.0 | [registry archive](https://registry.npmjs.org/lightningcss-win32-x64-msvc/-/lightningcss-win32-x64-msvc-1.33.0.tgz) |
| magic-string | 1.4.5 | MIT | [registry archive](https://registry.npmjs.org/magic-string/-/magic-string-1.4.5.tgz) |
| maplibre-gl | 4.7.1 | BSD-3-Clause | [registry archive](https://registry.npmjs.org/maplibre-gl/-/maplibre-gl-4.7.1.tgz) |
| minimist | 1.2.8 | MIT | [registry archive](https://registry.npmjs.org/minimist/-/minimist-1.2.8.tgz) |
| murmurhash-js | 1.0.0 | MIT | [registry archive](https://registry.npmjs.org/murmurhash-js/-/murmurhash-js-1.0.0.tgz) |
| nanoid | 3.3.20 | MIT | [registry archive](https://registry.npmjs.org/nanoid/-/nanoid-3.3.20.tgz) |
| obug | 3.0.0 | MIT | [registry archive](https://registry.npmjs.org/obug/-/obug-3.0.0.tgz) |
| pbf | 3.3.0 | BSD-3-Clause | [registry archive](https://registry.npmjs.org/pbf/-/pbf-3.3.0.tgz) |
| picocolors | 1.1.1 | ISC | [registry archive](https://registry.npmjs.org/picocolors/-/picocolors-1.1.1.tgz) |
| picomatch | 4.0.7 | MIT | [registry archive](https://registry.npmjs.org/picomatch/-/picomatch-4.0.7.tgz) |
| playwright-core | 1.65.0 | Apache-2.0 | [registry archive](https://registry.npmjs.org/playwright-core/-/playwright-core-1.65.0.tgz) |
| pmtiles | 4.4.0 | BSD-3-Clause | [registry archive](https://registry.npmjs.org/pmtiles/-/pmtiles-4.4.0.tgz) |
| postcss | 8.5.32 | MIT | [registry archive](https://registry.npmjs.org/postcss/-/postcss-8.5.32.tgz) |
| potpack | 2.1.0 | ISC | [registry archive](https://registry.npmjs.org/potpack/-/potpack-2.1.0.tgz) |
| protocol-buffers-schema | 3.6.1 | MIT | [registry archive](https://registry.npmjs.org/protocol-buffers-schema/-/protocol-buffers-schema-3.6.1.tgz) |
| quickselect | 3.0.0 | ISC | [registry archive](https://registry.npmjs.org/quickselect/-/quickselect-3.0.0.tgz) |
| react | 19.3.1 | MIT | [registry archive](https://registry.npmjs.org/react/-/react-19.3.1.tgz) |
| react-dom | 19.3.1 | MIT | [registry archive](https://registry.npmjs.org/react-dom/-/react-dom-19.3.1.tgz) |
| react-router | 7.18.5 | MIT | [registry archive](https://registry.npmjs.org/react-router/-/react-router-7.18.5.tgz) |
| react-router-dom | 7.18.5 | MIT | [registry archive](https://registry.npmjs.org/react-router-dom/-/react-router-dom-7.18.5.tgz) |
| resolve-protobuf-schema | 2.1.0 | MIT | [registry archive](https://registry.npmjs.org/resolve-protobuf-schema/-/resolve-protobuf-schema-2.1.0.tgz) |
| rolldown | 1.2.19 | MIT | [registry archive](https://registry.npmjs.org/rolldown/-/rolldown-1.2.19.tgz) |
| rw | 1.3.3 | BSD-3-Clause | [registry archive](https://registry.npmjs.org/rw/-/rw-1.3.3.tgz) |
| scheduler | 0.28.0 | MIT | [registry archive](https://registry.npmjs.org/scheduler/-/scheduler-0.28.0.tgz) |
| set-cookie-parser | 2.7.3 | MIT | [registry archive](https://registry.npmjs.org/set-cookie-parser/-/set-cookie-parser-2.7.3.tgz) |
| source-map-js | 1.2.2 | BSD-3-Clause | [registry archive](https://registry.npmjs.org/source-map-js/-/source-map-js-1.2.2.tgz) |
| std-env | 4.3.0 | MIT | [registry archive](https://registry.npmjs.org/std-env/-/std-env-4.3.0.tgz) |
| supercluster | 8.0.1 | ISC | [registry archive](https://registry.npmjs.org/supercluster/-/supercluster-8.0.1.tgz) |
| tailwindcss | 4.3.5 | MIT | [registry archive](https://registry.npmjs.org/tailwindcss/-/tailwindcss-4.3.5.tgz) |
| tapable | 2.3.3 | MIT | [registry archive](https://registry.npmjs.org/tapable/-/tapable-2.3.3.tgz) |
| tinybench | 6.2.3 | MIT | [registry archive](https://registry.npmjs.org/tinybench/-/tinybench-6.2.3.tgz) |
| tinyexec | 1.3.1 | MIT | [registry archive](https://registry.npmjs.org/tinyexec/-/tinyexec-1.3.1.tgz) |
| tinyglobby | 0.2.17 | MIT | [registry archive](https://registry.npmjs.org/tinyglobby/-/tinyglobby-0.2.17.tgz) |
| tinyqueue | 3.0.0 | ISC | [registry archive](https://registry.npmjs.org/tinyqueue/-/tinyqueue-3.0.0.tgz) |
| tslib | 2.9.1 | 0BSD | [registry archive](https://registry.npmjs.org/tslib/-/tslib-2.9.1.tgz) |
| vite | 8.3.8 | MIT | [registry archive](https://registry.npmjs.org/vite/-/vite-8.3.8.tgz) |
| vitest | 5.1.0 | MIT | [registry archive](https://registry.npmjs.org/vitest/-/vitest-5.1.0.tgz) |
| vt-pbf | 3.1.3 | MIT | [registry archive](https://registry.npmjs.org/vt-pbf/-/vt-pbf-3.1.3.tgz) |
| which | 4.0.0 | ISC | [registry archive](https://registry.npmjs.org/which/-/which-4.0.0.tgz) |
| why-is-node-running | 3.2.1 | MIT | [registry archive](https://registry.npmjs.org/why-is-node-running/-/why-is-node-running-3.2.1.tgz) |

Total: 149 locked dependency records.

The six optional WASM-package entries without their own `resolved` field are
bundled inside the linked `@tailwindcss/oxide-wasm32-wasi` archive. These source
links identify that containing archive, not a separate fetched package.

Newly bundled React UI license texts are retained verbatim in
`web-stub/public/assets/react-ui-NOTICES.txt` (SHA-256
`7269f07e5693acd0325fb5830d46d972aa1a8b920a013f1048c3fdb4b7b2f8b1`).
The file includes package/version, official locked archive URL and each
original text's SHA-256 for React, React DOM, scheduler, React Router/DOM,
TanStack Query core/React, TanStack Virtual core/React, cookie,
set-cookie-parser and Tailwind CSS. It ships under the same assets path.
Regenerate deterministically after a deliberate dependency change with
`node scripts/collect-ui-notices.mjs` from `web-stub`; this reads only locked
local packages, verifies installed versions and performs no network fetch.
The regular build copies this checked-in notice file without fetching it.

Unmodified Vite notice remains at `web-stub/public/assets/vite-LICENSE.txt`.
The generated map archive retains MapLibre, PMTiles and basemaps BSD notices,
the upstream Mapzen MIT icon notice and Noto SIL OFL 1.1. Exact component paths,
source URLs, byte counts and SHA-256 are recorded in `docs/spike/map-assets.toml`.
MapLibre source revision `87486a5ef2085e600e8fa4e31252629dd8488dcd`;
PMTiles source revision `6f6c9c8697f26c5394dc351f04cec08656e905b6`;
basemaps source revision `c4cac3d4b3b0c0b657bf61ebaa89659e83f4a362`;
Playwright source revision `a1f35e1955976e84411f9c10358228a80b763ebe`.
Its official package includes LICENSE, NOTICE and ThirdPartyNotices.txt; it
is absent from the production frontend entry graph. All npm installation used
`--ignore-scripts --no-audit --no-fund`.

Vitest package metadata was checked at https://registry.npmjs.org/vitest/5.1.0;
its installed LICENSE.md identifies the core MIT license and retains bundled notices.
Task 6A Step 1's 20 dependency additions are also listed in
`docs/spike/perf-helper-validation.md`. Vitest is absent from the production entry graph.

Fixture authoring uses the official Python `libzim` **4.1.0** Windows wheel
(GPL-3.0-or-later), source revision
`cf35f8a69e7c84ccc38d2faca31a3539005a9d2f`. This isolated development tool is
not bundled with the Rust app. The verified artifact, retained license location,
reported native component versions and original fixture-content provenance are
recorded in [article-fixtures.md](docs/spike/article-fixtures.md). Its native
component versions are a runtime receipt, not a separate native license audit.

## Task 7 selected ISO reader (pending build validation)

`libmkiso` **0.1.5**, MIT, upstream commit
`e5d8e75bbf9dd5dd33c0c6b12a9a0686efe916e9`, is selected as a reader-only local
safety fork (`default-features = false`). The published 247,321-byte crate
matched registry SHA-256
`262387b6106a83fc5a389eba8a513fee4024ef83e60f22aff8a3bbe462b03289`.
Its actual LICENSE credits 2026 windows-uup contributors; retain that complete
license in the fork. Its reader dependency is thiserror 2, already pinned above.
The core lockfile resolves the local 0.1.5 reader against the existing
thiserror 2.0.21; writer/CLI features are disabled. The excluded upstream package
retains its separate published development lockfile for upstream checks.
Source/license checks, alternatives and the required UDF arithmetic safety
patch are recorded in [iso-decision.md](docs/spike/iso-decision.md) and the fork's
provenance record. This selection is not a passing ISO gate.

Fixture-only checker: pycdlib **1.22.0**, LGPL-2.1-only, verified wheel SHA-256
`bfa0b3631345a14b2741fe088398b6b8dc2838442ea607fd07d027dcc67b9e94`.
It is installed in a user-local isolated environment and is not linked into the
app. Its complete COPYING text is retained at
`crates/core/tests/fixtures/iso/notices/pycdlib-1.22.0-COPYING.txt`.
Existing GNU xorriso 1.5.8.pl02 (GPL-3.0-or-later) and native Windows IMAPI2FS
10.0.26100.1 (installed OS license, not redistributed) author the fixtures.
Exact artifacts, checker limitations and source payload licenses are recorded
in `iso-fixtures-validation.md` and `iso-fixtures-receipt.json`.

## LAN spike additions (2026-11-11)

Exact direct pins: rustls 0.23.46 (ring/std/tls12), tokio-rustls 0.26.6
(ring/tls12), rcgen 0.14.10 (ring/zeroize; defaults off), mdns-sd 0.21.7
(defaults off), Hyper 1.12.0 and hyper-util 0.1.21. Already recorded packages
retain their entries above. The following new lockfile entries were checked
against downloaded Cargo manifests and packaged license declarations/texts;
this is the resolved inventory, not a claim that every optional package ships
in every target. Android linkage/notice refresh remains part of its native build.

| Package | Exact version | Declared license | Source |
| --- | --- | --- | --- |
| asn1-rs | 0.7.2 | MIT OR Apache-2.0 | [upstream](https://github.com/rusticata/asn1-rs) |
| asn1-rs-derive | 0.6.0 | MIT OR Apache-2.0 | [upstream](https://github.com/rusticata/asn1-rs) |
| asn1-rs-impl | 0.2.0 | MIT/Apache-2.0 | [upstream](https://github.com/rusticata/asn1-rs) |
| bit-vec | 0.9.1 | Apache-2.0 OR MIT | [upstream](https://github.com/contain-rs/bit-vec) |
| data-encoding | 2.11.1 | MIT | [upstream](https://github.com/ia0/data-encoding) |
| der-parser | 10.0.0 | MIT OR Apache-2.0 | [upstream](https://github.com/rusticata/der-parser) |
| flume | 0.12.0 | Apache-2.0/MIT | [upstream](https://github.com/zesterer/flume) |
| if-addrs | 0.15.0 | MIT OR BSD-3-Clause | [upstream](https://github.com/messense/if-addrs) |
| lazy_static | 1.5.1 | MIT OR Apache-2.0 | [upstream](https://github.com/rust-lang-nursery/lazy-static.rs) |
| mdns-sd | 0.21.7 | Apache-2.0 OR MIT | [upstream](https://github.com/keepsimple1/mdns-sd) |
| num-bigint | 0.4.8 | MIT OR Apache-2.0 | [upstream](https://github.com/rust-num/num-bigint) |
| num-integer | 0.1.47 | MIT OR Apache-2.0 | [upstream](https://github.com/rust-num/num-integer) |
| oid-registry | 0.8.1 | MIT OR Apache-2.0 | [upstream](https://github.com/rusticata/oid-registry) |
| rcgen | 0.14.10 | MIT OR Apache-2.0 | [upstream](https://github.com/rustls/rcgen) |
| rusticata-macros | 4.1.0 | MIT/Apache-2.0 | [upstream](https://github.com/rusticata/rusticata-macros) |
| socket-pktinfo | 0.4.1 | MIT | [upstream](https://github.com/pixsper/socket-pktinfo) |
| spin | 0.9.9 | MIT | [upstream](https://github.com/mvdnes/spin-rs) |
| synstructure | 0.13.2 | MIT | [upstream](https://github.com/mystor/synstructure) |
| x509-parser | 0.18.1 | MIT OR Apache-2.0 | [upstream](https://github.com/rusticata/x509-parser) |
| yasna | 0.6.0 | MIT OR Apache-2.0 | [upstream](https://github.com/qnighy/yasna.rs) |

Slash-separated dual-license declarations above preserve their original legacy
Cargo spelling. `asn1-rs-impl` packages its declaration without standalone
license files; its upstream workspace carries the MIT/Apache texts. rcgen's
packaged LICENSE declares the dual grant. Do not substitute a different version
or infer native-target linkage from this table alone.

## Selected ZIM content

| Archive | Version | Checked scope |
| --- | --- | --- |
| English Wikipedia Medicine nopic / WikiMed Medical Encyclopedia |2026-10; exact bytes/SHA in `docs/spike/packs.toml`| Publisher pin and local archive SHA match. Three authentic HTML pages, including the landing page, identify text as CC BY-SA4.0 unless otherwise noted, with separate media terms. This is not a blanket asset-license audit. |

Preserve the archive's source links, attribution and notices. Bounded original
HTML/footer evidence is in `target/medicine-metadata-r1/legal/`; the absent
License metadata field does not override those page notices. Remaining content
exceptions and packaged media terms stay open.

## Original plan candidates (historical)

These initial candidates are retained for provenance. Their TBD cells are not
current pins; checked dependency/model records elsewhere in this file supersede
them. Unselected content still needs its own verification.

| Dependency | Candidate version | Status |
| --- | --- | --- |
| qrcode (LAN dependencies resolved above) | TBD | Unverified |
| llama-cpp-2, llama.cpp, models | TBD | Unverified |
| Tauri (installed React/Tailwind/router/query/virtual versions are recorded above) | TBD | Unverified |
| Additional map packs and OpenStreetMap data | Per pack | Content terms must be verified per selected dataset; installed renderer/assets are recorded above. |
| Kiwix/Wikipedia and other ZIM content | TBD | Unverified per pack |

Native transitive dependencies, model and content licenses must be recorded
when selected. No design-document compatibility claim is treated as verified.

## Task 8 Android and UniFFI dependencies

UniFFI 0.32.2 is pinned from the verified registry release; its MPL-2.0
license was checked against installed source. Core enables the optional ffi
feature with UniFFI default features disabled. Bindgen/CLI dependencies belong
to the separate host tooling feature, not the Android runtime library.
The 34 added registry package/version/license declarations below were read
from the stable Cargo.lock and downloaded Cargo manifests after resolution.
This is an exact declaration inventory, not a separate audit of every file.

| Package | Exact version | Declared license |
| --- | --- | --- |
| `anstyle` | `1.0.14` | MIT OR Apache-2.0 |
| `anyhow` | `1.0.104` | MIT OR Apache-2.0 |
| `askama` | `0.16.1` | MIT OR Apache-2.0 |
| `askama_derive` | `0.16.1` | MIT OR Apache-2.0 |
| `askama_macros` | `0.16.1` | MIT OR Apache-2.0 |
| `askama_parser` | `0.16.1` | MIT OR Apache-2.0 |
| `basic-toml` | `0.1.10` | MIT OR Apache-2.0 |
| `camino` | `1.2.6` | MIT OR Apache-2.0 |
| `clap` | `4.6.7` | MIT OR Apache-2.0 |
| `clap_builder` | `4.6.7` | MIT OR Apache-2.0 |
| `clap_derive` | `4.6.7` | MIT OR Apache-2.0 |
| `clap_lex` | `1.1.1` | MIT OR Apache-2.0 |
| `fs-err` | `3.3.2` | MIT OR Apache-2.0 |
| `goblin` | `0.8.2` | MIT |
| `heck` | `0.5.0` | MIT OR Apache-2.0 |
| `plain` | `0.2.3` | MIT/Apache-2.0 |
| `scroll` | `0.12.0` | MIT |
| `scroll_derive` | `0.12.1` | MIT |
| `serde_spanned` | `1.1.3` | MIT OR Apache-2.0 |
| `smawk` | `0.3.3` | MIT |
| `static_assertions` | `1.1.0` | MIT OR Apache-2.0 |
| `textwrap` | `0.16.4` | MIT |
| `toml` | `1.2.1+spec-1.1.0` | MIT OR Apache-2.0 |
| `toml_writer` | `1.2.0+spec-1.1.0` | MIT OR Apache-2.0 |
| `unicode-width` | `0.2.3` | MIT OR Apache-2.0 |
| `uniffi` | `0.32.2` | MPL-2.0 |
| `uniffi_bindgen` | `0.32.2` | MPL-2.0 |
| `uniffi_core` | `0.32.2` | MPL-2.0 |
| `uniffi_internal_macros` | `0.32.2` | MPL-2.0 |
| `uniffi_macros` | `0.32.2` | MPL-2.0 |
| `uniffi_meta` | `0.32.2` | MPL-2.0 |
| `uniffi_pipeline` | `0.32.2` | MPL-2.0 |
| `uniffi_udl` | `0.32.2` | MPL-2.0 |
| `weedle2` | `5.0.0` | MIT |

Android runtime dependencies: Kotlin stdlib 2.1.21 (Apache-2.0), JetBrains
annotations 13.0 (Apache-2.0), JNA Android AAR 5.17.0 (Apache-2.0 option
selected from its dual license), including statically bundled libffi 3.4.4
(MIT). Exact source/POM checks, AAR/native hashes and retained license/source
notices are recorded in android-readiness.md and android-shell-validation.md.
Complete JNA/libffi notices are retained under android/licenses and packaged
with the root GPL, NOTICE and this inventory as APK assets.

Build-only tools: Gradle 8.9, Android Gradle Plugin 8.7.2 and Kotlin Gradle
plugin 2.1.21 (Apache-2.0). Wrapper/distribution hashes and resolved Gradle
dependency lock/SHA-256 verification metadata are retained under android/.
Compile-only annotations: androidx.annotation:annotation 1.9.1 and its
annotation-jvm 1.9.1 variant (Apache-2.0, exact Google Maven POM checked).
Generated UniFFI Android bindings require these binary-retained annotations;
they do not add an AndroidX runtime framework. Their older Kotlin requirement
resolves to the already pinned stdlib 2.1.21.
The resolved runtime graph contains the three Java/Kotlin dependencies above;
Gradle plugin transitives are recorded in its lock and verification files.
SDK/NDK/JDK are existing host tools with the provenance described in build
records; they are not additional APK runtime frameworks.

### Complete Gradle buildgraph declaration inventory

The 256 exact verification-metadata components below include build plugins,
lint/test tools, POM-only parents/BOMs and version candidates, not just APK
runtime. All 99 lockfile coordinates are covered. There are 249 checked
declarations and seven explicitly Unknown entries; missing declarations are
not silently inferred. Detailed exact POM/source links and method are retained
in docs/spike/android-gradle-licenses.md. These buildgraph entries are separate
from the previously identified three Java/Kotlin APK runtime dependencies.

| Exact group:artifact:version | Declared license(s) |
| --- | --- |
| `androidx.annotation:annotation:1.9.1` | The Apache Software License, Version 2.0 |
| `androidx.annotation:annotation-jvm:1.9.1` | The Apache Software License, Version 2.0 |
| `androidx.databinding:databinding-common:8.7.2` | The Apache Software License, Version 2.0 |
| `androidx.databinding:databinding-compiler-common:8.7.2` | The Apache Software License, Version 2.0 |
| `com.android:signflinger:8.7.2` | The Apache Software License, Version 2.0 |
| `com.android:zipflinger:8.7.2` | The Apache Software License, Version 2.0 |
| `com.android.application:com.android.application.gradle.plugin:8.7.2` | The Apache Software License, Version 2.0 |
| `com.android.databinding:baseLibrary:8.7.2` | The Apache Software License, Version 2.0 |
| `com.android.tools:annotations:31.7.2` | The Apache Software License, Version 2.0 |
| `com.android.tools:common:31.7.2` | The Apache Software License, Version 2.0 |
| `com.android.tools:dvlib:31.7.2` | The Apache Software License, Version 2.0 |
| `com.android.tools:play-sdk-proto:31.7.2` | The Apache Software License, Version 2.0 |
| `com.android.tools:repository:31.7.2` | The Apache Software License, Version 2.0 |
| `com.android.tools:sdk-common:31.7.2` | The Apache Software License, Version 2.0 |
| `com.android.tools:sdklib:31.7.2` | The Apache Software License, Version 2.0 |
| `com.android.tools.analytics-library:crash:31.7.2` | The Apache Software License, Version 2.0 |
| `com.android.tools.analytics-library:protos:31.7.2` | The Apache Software License, Version 2.0 |
| `com.android.tools.analytics-library:shared:31.7.2` | The Apache Software License, Version 2.0 |
| `com.android.tools.analytics-library:tracker:31.7.2` | The Apache Software License, Version 2.0 |
| `com.android.tools.build:aapt2:8.7.2-12006047` | The Apache Software License, Version 2.0 |
| `com.android.tools.build:aapt2-proto:8.7.2-12006047` | The Apache Software License, Version 2.0 |
| `com.android.tools.build:aaptcompiler:8.7.2` | The Apache Software License, Version 2.0 |
| `com.android.tools.build:apksig:8.7.2` | The Apache Software License, Version 2.0 |
| `com.android.tools.build:apkzlib:8.7.2` | The Apache Software License, Version 2.0 |
| `com.android.tools.build:builder:8.7.2` | The Apache Software License, Version 2.0 |
| `com.android.tools.build:builder-model:8.7.2` | The Apache Software License, Version 2.0 |
| `com.android.tools.build:builder-test-api:8.7.2` | The Apache Software License, Version 2.0 |
| `com.android.tools.build:bundletool:1.17.1` | The Apache Software License, Version 2.0 |
| `com.android.tools.build:gradle:8.7.2` | The Apache Software License, Version 2.0 |
| `com.android.tools.build:gradle-api:8.7.2` | The Apache Software License, Version 2.0 |
| `com.android.tools.build:gradle-settings-api:8.7.2` | The Apache Software License, Version 2.0 |
| `com.android.tools.build:manifest-merger:31.7.2` | The Apache Software License, Version 2.0 |
| `com.android.tools.build:transform-api:2.0.0-deprecated-use-gradle-api` | The Apache Software License, Version 2.0 |
| `com.android.tools.build.jetifier:jetifier-core:1.0.0-beta10` | The Apache Software License, Version 2.0 |
| `com.android.tools.build.jetifier:jetifier-processor:1.0.0-beta10` | The Apache Software License, Version 2.0 |
| `com.android.tools.ddms:ddmlib:31.7.2` | The Apache Software License, Version 2.0 |
| `com.android.tools.emulator:proto:31.7.2` | The Apache Software License, Version 2.0 |
| `com.android.tools.external.com-intellij:intellij-core:31.7.2` | The Apache Software License, Version 2.0 |
| `com.android.tools.external.com-intellij:kotlin-compiler:31.7.2` | The Apache Software License, Version 2.0 |
| `com.android.tools.external.org-jetbrains:uast:31.7.2` | The Apache Software License, Version 2.0 |
| `com.android.tools.layoutlib:layoutlib-api:31.7.2` | The Apache Software License, Version 2.0 |
| `com.android.tools.lint:lint:31.7.2` | The Apache Software License, Version 2.0 |
| `com.android.tools.lint:lint-api:31.7.2` | The Apache Software License, Version 2.0 |
| `com.android.tools.lint:lint-checks:31.7.2` | The Apache Software License, Version 2.0 |
| `com.android.tools.lint:lint-gradle:31.7.2` | The Apache Software License, Version 2.0 |
| `com.android.tools.lint:lint-model:31.7.2` | The Apache Software License, Version 2.0 |
| `com.android.tools.lint:lint-typedef-remover:31.7.2` | The Apache Software License, Version 2.0 |
| `com.android.tools.utp:android-device-provider-ddmlib:31.7.2` | The Apache Software License, Version 2.0 |
| `com.android.tools.utp:android-device-provider-ddmlib-proto:31.7.2` | The Apache Software License, Version 2.0 |
| `com.android.tools.utp:android-device-provider-gradle:31.7.2` | The Apache Software License, Version 2.0 |
| `com.android.tools.utp:android-device-provider-gradle-proto:31.7.2` | The Apache Software License, Version 2.0 |
| `com.android.tools.utp:android-device-provider-profile:31.7.2` | The Apache Software License, Version 2.0 |
| `com.android.tools.utp:android-device-provider-profile-proto:31.7.2` | The Apache Software License, Version 2.0 |
| `com.android.tools.utp:android-test-plugin-host-additional-test-output:31.7.2` | The Apache Software License, Version 2.0 |
| `com.android.tools.utp:android-test-plugin-host-additional-test-output-proto:31.7.2` | The Apache Software License, Version 2.0 |
| `com.android.tools.utp:android-test-plugin-host-apk-installer:31.7.2` | The Apache Software License, Version 2.0 |
| `com.android.tools.utp:android-test-plugin-host-apk-installer-proto:31.7.2` | The Apache Software License, Version 2.0 |
| `com.android.tools.utp:android-test-plugin-host-coverage:31.7.2` | The Apache Software License, Version 2.0 |
| `com.android.tools.utp:android-test-plugin-host-coverage-proto:31.7.2` | The Apache Software License, Version 2.0 |
| `com.android.tools.utp:android-test-plugin-host-device-info:31.7.2` | The Apache Software License, Version 2.0 |
| `com.android.tools.utp:android-test-plugin-host-device-info-proto:31.7.2` | The Apache Software License, Version 2.0 |
| `com.android.tools.utp:android-test-plugin-host-emulator-control:31.7.2` | The Apache Software License, Version 2.0 |
| `com.android.tools.utp:android-test-plugin-host-emulator-control-proto:31.7.2` | The Apache Software License, Version 2.0 |
| `com.android.tools.utp:android-test-plugin-host-logcat:31.7.2` | The Apache Software License, Version 2.0 |
| `com.android.tools.utp:android-test-plugin-host-logcat-proto:31.7.2` | The Apache Software License, Version 2.0 |
| `com.android.tools.utp:android-test-plugin-host-retention:31.7.2` | The Apache Software License, Version 2.0 |
| `com.android.tools.utp:android-test-plugin-host-retention-proto:31.7.2` | The Apache Software License, Version 2.0 |
| `com.android.tools.utp:android-test-plugin-result-listener-gradle:31.7.2` | The Apache Software License, Version 2.0 |
| `com.android.tools.utp:android-test-plugin-result-listener-gradle-proto:31.7.2` | The Apache Software License, Version 2.0 |
| `com.android.tools.utp:utp-common:31.7.2` | The Apache Software License, Version 2.0 |
| `com.google.android:annotations:4.1.1.4` | Apache 2.0 |
| `com.google.api.grpc:proto-google-common-protos:2.17.0` | Apache-2.0 |
| `com.google.auto:auto-parent:6` | Apache 2.0 |
| `com.google.auto.value:auto-value-annotations:1.6.2` | Apache 2.0 |
| `com.google.auto.value:auto-value-parent:1.6.2` | Apache 2.0 |
| `com.google.code.findbugs:jsr305:3.0.2` | The Apache Software License, Version 2.0 |
| `com.google.code.gson:gson:2.10.1` | Apache-2.0 |
| `com.google.code.gson:gson:2.11.0` | Apache-2.0 |
| `com.google.code.gson:gson-parent:2.10.1` | Apache-2.0 |
| `com.google.code.gson:gson-parent:2.11.0` | Apache-2.0 |
| `com.google.crypto.tink:tink:1.7.0` | Apache License, Version 2.0 |
| `com.google.dagger:dagger:2.28.3` | Apache 2.0 |
| `com.google.errorprone:error_prone_annotations:2.11.0` | Apache 2.0 |
| `com.google.errorprone:error_prone_annotations:2.18.0` | Apache 2.0 |
| `com.google.errorprone:error_prone_annotations:2.27.0` | Apache 2.0 |
| `com.google.errorprone:error_prone_annotations:2.3.1` | Apache 2.0 |
| `com.google.errorprone:error_prone_parent:2.11.0` | Apache 2.0 |
| `com.google.errorprone:error_prone_parent:2.18.0` | Apache 2.0 |
| `com.google.errorprone:error_prone_parent:2.27.0` | Apache 2.0 |
| `com.google.errorprone:error_prone_parent:2.3.1` | Apache 2.0 |
| `com.google.flatbuffers:flatbuffers-java:1.12.0` | Apache License V2.0 |
| `com.google.guava:failureaccess:1.0.1` | The Apache Software License, Version 2.0 |
| `com.google.guava:guava:32.0.1-jre` | Apache License, Version 2.0 |
| `com.google.guava:guava-parent:26.0-android` | The Apache Software License, Version 2.0 |
| `com.google.guava:guava-parent:32.0.1-jre` | Apache License, Version 2.0 |
| `com.google.guava:listenablefuture:9999.0-empty-to-avoid-conflict-with-guava` | The Apache Software License, Version 2.0 |
| `com.google.j2objc:j2objc-annotations:1.3` | The Apache Software License, Version 2.0 |
| `com.google.j2objc:j2objc-annotations:2.8` | Apache License, Version 2.0 |
| `com.google.jimfs:jimfs:1.1` | The Apache Software License, Version 2.0 |
| `com.google.jimfs:jimfs-parent:1.1` | The Apache Software License, Version 2.0 |
| `com.google.protobuf:protobuf-bom:3.22.3` | BSD-3-Clause |
| `com.google.protobuf:protobuf-java:3.22.3` | BSD-3-Clause |
| `com.google.protobuf:protobuf-java-util:3.22.3` | BSD-3-Clause |
| `com.google.protobuf:protobuf-parent:3.22.3` | BSD-3-Clause |
| `com.google.testing.platform:android-device-provider-local:0.0.9-alpha02` | Android Software Development Kit License Agreement |
| `com.google.testing.platform:android-driver-instrumentation:0.0.9-alpha02` | Android Software Development Kit License Agreement |
| `com.google.testing.platform:android-test-plugin:0.0.9-alpha02` | Android Software Development Kit License Agreement |
| `com.google.testing.platform:core:0.0.9-alpha02` | Android Software Development Kit License Agreement |
| `com.google.testing.platform:core-proto:0.0.9-alpha02` | The Apache Software License, Version 2.0 |
| `com.google.testing.platform:launcher:0.0.9-alpha02` | Android Software Development Kit License Agreement |
| `com.googlecode.juniversalchardet:juniversalchardet:1.0.3` | Mozilla Public License 1.1 (MPL 1.1) |
| `com.squareup:javapoet:1.10.0` | Apache 2.0 |
| `com.squareup:javawriter:2.5.0` | Apache 2.0 |
| `com.sun.activation:all:1.2.0` | CDDL/GPLv2+CE |
| `com.sun.activation:all:1.2.1` | EDL 1.0 |
| `com.sun.activation:javax.activation:1.2.0` | CDDL/GPLv2+CE |
| `com.sun.istack:istack-commons:3.0.8` | Eclipse Distribution License - v 1.0 |
| `com.sun.istack:istack-commons-runtime:3.0.8` | Eclipse Distribution License - v 1.0 |
| `com.sun.xml.bind:jaxb-bom-ext:2.3.2` | Eclipse Public License v. 2.0; GNU General Public License, version 2 with the GNU Classpath Exception |
| `com.sun.xml.bind.mvn:jaxb-parent:2.3.2` | Eclipse Distribution License - v 1.0 |
| `com.sun.xml.bind.mvn:jaxb-runtime-parent:2.3.2` | Eclipse Distribution License - v 1.0 |
| `com.sun.xml.bind.mvn:jaxb-txw-parent:2.3.2` | Eclipse Distribution License - v 1.0 |
| `com.sun.xml.fastinfoset:FastInfoset:1.2.16` | Apache License, Version 2.0; Eclipse Distribution License - v 1.0 |
| `com.sun.xml.fastinfoset:fastinfoset-project:1.2.16` | Apache License, Version 2.0; Eclipse Distribution License - v 1.0 |
| `commons-codec:commons-codec:1.10` | Apache License, Version 2.0 |
| `commons-codec:commons-codec:1.11` | Apache License, Version 2.0 |
| `commons-io:commons-io:2.13.0` | Apache-2.0 |
| `commons-logging:commons-logging:1.2` | The Apache Software License, Version 2.0 |
| `io.grpc:grpc-api:1.57.0` | Apache 2.0 |
| `io.grpc:grpc-context:1.57.0` | Apache 2.0 |
| `io.grpc:grpc-core:1.57.0` | Apache 2.0 |
| `io.grpc:grpc-netty:1.57.0` | Apache 2.0 |
| `io.grpc:grpc-protobuf:1.57.0` | Apache 2.0 |
| `io.grpc:grpc-protobuf-lite:1.57.0` | Apache 2.0 |
| `io.grpc:grpc-stub:1.57.0` | Apache 2.0 |
| `io.netty:netty-buffer:4.1.93.Final` | Apache License, Version 2.0 |
| `io.netty:netty-codec:4.1.93.Final` | Apache License, Version 2.0 |
| `io.netty:netty-codec-http:4.1.93.Final` | Apache License, Version 2.0 |
| `io.netty:netty-codec-http2:4.1.93.Final` | Apache License, Version 2.0 |
| `io.netty:netty-codec-socks:4.1.93.Final` | Apache License, Version 2.0 |
| `io.netty:netty-common:4.1.93.Final` | Apache License, Version 2.0 |
| `io.netty:netty-handler:4.1.93.Final` | Apache License, Version 2.0 |
| `io.netty:netty-handler-proxy:4.1.93.Final` | Apache License, Version 2.0 |
| `io.netty:netty-parent:4.1.93.Final` | Apache License, Version 2.0 |
| `io.netty:netty-resolver:4.1.93.Final` | Apache License, Version 2.0 |
| `io.netty:netty-transport:4.1.93.Final` | Apache License, Version 2.0 |
| `io.netty:netty-transport-native-unix-common:4.1.93.Final` | Apache License, Version 2.0 |
| `io.perfmark:perfmark-api:0.26.0` | Apache 2.0 |
| `jakarta.activation:jakarta.activation-api:1.2.1` | EDL 1.0 |
| `jakarta.xml.bind:jakarta.xml.bind-api:2.3.2` | Eclipse Distribution License - v 1.0 |
| `jakarta.xml.bind:jakarta.xml.bind-api-parent:2.3.2` | Eclipse Distribution License - v 1.0 |
| `javax.annotation:javax.annotation-api:1.3.2` | CDDL + GPLv2 with classpath exception |
| `javax.inject:javax.inject:1` | **Unknown** |
| `net.java:jvnet-parent:1` | **Unknown** |
| `net.java:jvnet-parent:3` | **Unknown** |
| `net.java.dev.jna:jna:5.17.0` | LGPL-2.1-or-later; Apache-2.0 |
| `net.java.dev.jna:jna:5.6.0` | LGPL, version 2.1; Apache License v2.0 |
| `net.java.dev.jna:jna-platform:5.6.0` | LGPL, version 2.1; Apache License v2.0 |
| `net.sf.jopt-simple:jopt-simple:4.9` | The MIT License |
| `net.sf.kxml:kxml2:2.3.0` | **Unknown** |
| `org.apache:apache:13` | The Apache Software License, Version 2.0 |
| `org.apache:apache:15` | Apache License, Version 2.0 |
| `org.apache:apache:18` | Apache License, Version 2.0 |
| `org.apache:apache:21` | Apache License, Version 2.0 |
| `org.apache:apache:23` | Apache License, Version 2.0 |
| `org.apache:apache:29` | Apache-2.0 |
| `org.apache.commons:commons-compress:1.21` | Apache License, Version 2.0 |
| `org.apache.commons:commons-parent:34` | The Apache Software License, Version 2.0 |
| `org.apache.commons:commons-parent:35` | Apache License, Version 2.0 |
| `org.apache.commons:commons-parent:42` | Apache License, Version 2.0 |
| `org.apache.commons:commons-parent:52` | Apache License, Version 2.0 |
| `org.apache.commons:commons-parent:58` | Apache-2.0 |
| `org.apache.httpcomponents:httpclient:4.5.14` | Apache License, Version 2.0 |
| `org.apache.httpcomponents:httpclient:4.5.6` | Apache License, Version 2.0 |
| `org.apache.httpcomponents:httpcomponents-client:4.5.14` | Apache License, Version 2.0 |
| `org.apache.httpcomponents:httpcomponents-client:4.5.6` | Apache License, Version 2.0 |
| `org.apache.httpcomponents:httpcomponents-core:4.4.16` | Apache License, Version 2.0 |
| `org.apache.httpcomponents:httpcomponents-parent:10` | Apache License, Version 2.0 |
| `org.apache.httpcomponents:httpcomponents-parent:11` | Apache License, Version 2.0 |
| `org.apache.httpcomponents:httpcore:4.4.16` | Apache License, Version 2.0 |
| `org.apache.httpcomponents:httpmime:4.5.6` | Apache License, Version 2.0 |
| `org.bitbucket.b_c:jose4j:0.9.5` | The Apache Software License, Version 2.0 |
| `org.bouncycastle:bcpkix-jdk18on:1.77` | MIT |
| `org.bouncycastle:bcprov-jdk18on:1.77` | MIT |
| `org.bouncycastle:bcutil-jdk18on:1.77` | MIT |
| `org.checkerframework:checker-qual:2.5.8` | The MIT License |
| `org.checkerframework:checker-qual:3.33.0` | The MIT License |
| `org.codehaus.groovy:groovy:3.0.21` | The Apache Software License, Version 2.0 |
| `org.codehaus.mojo:animal-sniffer-annotations:1.23` | MIT license |
| `org.codehaus.mojo:animal-sniffer-parent:1.23` | MIT license |
| `org.codehaus.mojo:mojo-parent:74` | The Apache Software License, Version 2.0 |
| `org.eclipse.ee4j:project:1.0.2` | Eclipse Public License v. 2.0; GNU General Public License, version 2 with the GNU Classpath Exception |
| `org.eclipse.ee4j:project:1.0.5` | Eclipse Public License v. 2.0; GNU General Public License, version 2 with the GNU Classpath Exception |
| `org.glassfish.jaxb:jaxb-bom:2.3.2` | Eclipse Public License v. 2.0; GNU General Public License, version 2 with the GNU Classpath Exception |
| `org.glassfish.jaxb:jaxb-runtime:2.3.2` | Eclipse Distribution License - v 1.0 |
| `org.glassfish.jaxb:txw2:2.3.2` | Eclipse Distribution License - v 1.0 |
| `org.jdom:jdom2:2.0.6` | JDOM license (four clauses, custom BSD-style) |
| `org.jetbrains:annotations:13.0` | The Apache Software License, Version 2.0 |
| `org.jetbrains:annotations:23.0.0` | The Apache Software License, Version 2.0 |
| `org.jetbrains.intellij.deps:trove4j:1.0.20200330` | GNU LESSER GENERAL PUBLIC LICENSE 2.1 |
| `org.jetbrains.kotlin:fus-statistics-gradle-plugin:2.1.21` | The Apache License, Version 2.0 |
| `org.jetbrains.kotlin:kotlin-build-statistics:2.1.21` | The Apache License, Version 2.0 |
| `org.jetbrains.kotlin:kotlin-build-tools-api:2.1.21` | The Apache License, Version 2.0 |
| `org.jetbrains.kotlin:kotlin-build-tools-impl:2.1.21` | The Apache License, Version 2.0 |
| `org.jetbrains.kotlin:kotlin-compiler-embeddable:2.1.21` | The Apache License, Version 2.0 |
| `org.jetbrains.kotlin:kotlin-compiler-runner:2.1.21` | The Apache License, Version 2.0 |
| `org.jetbrains.kotlin:kotlin-daemon-client:2.1.21` | The Apache License, Version 2.0 |
| `org.jetbrains.kotlin:kotlin-daemon-embeddable:2.1.21` | The Apache License, Version 2.0 |
| `org.jetbrains.kotlin:kotlin-gradle-plugin:2.1.21` | The Apache License, Version 2.0 |
| `org.jetbrains.kotlin:kotlin-gradle-plugin-annotations:2.1.21` | The Apache License, Version 2.0 |
| `org.jetbrains.kotlin:kotlin-gradle-plugin-api:2.1.21` | The Apache License, Version 2.0 |
| `org.jetbrains.kotlin:kotlin-gradle-plugin-idea:2.1.21` | The Apache License, Version 2.0 |
| `org.jetbrains.kotlin:kotlin-gradle-plugin-idea-proto:2.1.21` | The Apache License, Version 2.0 |
| `org.jetbrains.kotlin:kotlin-gradle-plugin-model:2.1.21` | The Apache License, Version 2.0 |
| `org.jetbrains.kotlin:kotlin-gradle-plugins-bom:2.1.21` | The Apache License, Version 2.0 |
| `org.jetbrains.kotlin:kotlin-klib-commonizer-api:2.1.21` | The Apache License, Version 2.0 |
| `org.jetbrains.kotlin:kotlin-klib-commonizer-embeddable:2.1.21` | The Apache License, Version 2.0 |
| `org.jetbrains.kotlin:kotlin-native-utils:2.1.21` | The Apache License, Version 2.0 |
| `org.jetbrains.kotlin:kotlin-reflect:1.6.10` | The Apache License, Version 2.0 |
| `org.jetbrains.kotlin:kotlin-reflect:1.9.20` | The Apache License, Version 2.0 |
| `org.jetbrains.kotlin:kotlin-reflect:1.9.23` | The Apache License, Version 2.0 |
| `org.jetbrains.kotlin:kotlin-script-runtime:2.1.21` | The Apache License, Version 2.0 |
| `org.jetbrains.kotlin:kotlin-scripting-common:2.1.21` | The Apache License, Version 2.0 |
| `org.jetbrains.kotlin:kotlin-scripting-compiler-embeddable:2.1.21` | The Apache License, Version 2.0 |
| `org.jetbrains.kotlin:kotlin-scripting-compiler-impl-embeddable:2.1.21` | The Apache License, Version 2.0 |
| `org.jetbrains.kotlin:kotlin-scripting-jvm:2.1.21` | The Apache License, Version 2.0 |
| `org.jetbrains.kotlin:kotlin-stdlib:1.9.20` | The Apache License, Version 2.0 |
| `org.jetbrains.kotlin:kotlin-stdlib:1.9.23` | The Apache License, Version 2.0 |
| `org.jetbrains.kotlin:kotlin-stdlib:1.9.24` | The Apache License, Version 2.0 |
| `org.jetbrains.kotlin:kotlin-stdlib:2.1.21` | The Apache License, Version 2.0 |
| `org.jetbrains.kotlin:kotlin-stdlib-common:1.9.20` | The Apache License, Version 2.0 |
| `org.jetbrains.kotlin:kotlin-stdlib-jdk7:1.9.20` | The Apache License, Version 2.0 |
| `org.jetbrains.kotlin:kotlin-stdlib-jdk8:1.9.20` | The Apache License, Version 2.0 |
| `org.jetbrains.kotlin:kotlin-tooling-core:2.1.21` | The Apache License, Version 2.0 |
| `org.jetbrains.kotlin:kotlin-util-io:2.1.21` | The Apache License, Version 2.0 |
| `org.jetbrains.kotlin:kotlin-util-klib:2.1.21` | The Apache License, Version 2.0 |
| `org.jetbrains.kotlin:kotlin-util-klib-metadata:2.1.21` | The Apache License, Version 2.0 |
| `org.jetbrains.kotlin.android:org.jetbrains.kotlin.android.gradle.plugin:2.1.21` | The Apache License, Version 2.0 |
| `org.jetbrains.kotlinx:kotlinx-coroutines-bom:1.6.4` | The Apache Software License, Version 2.0 |
| `org.jetbrains.kotlinx:kotlinx-coroutines-core:1.6.4` | The Apache Software License, Version 2.0 |
| `org.jetbrains.kotlinx:kotlinx-coroutines-core-jvm:1.6.4` | The Apache Software License, Version 2.0 |
| `org.jetbrains.kotlinx:kotlinx-coroutines-core-jvm:1.8.0` | The Apache Software License, Version 2.0 |
| `org.junit:junit-bom:5.9.2` | Eclipse Public License v2.0 |
| `org.junit:junit-bom:5.9.3` | Eclipse Public License v2.0 |
| `org.jvnet.staxex:stax-ex:1.8.1` | Eclipse Distribution License - v 1.0 |
| `org.ow2:ow2:1.5.1` | The Apache Software License, Version 2.0 |
| `org.ow2.asm:asm:9.6` | BSD-3-Clause |
| `org.ow2.asm:asm-analysis:9.6` | BSD-3-Clause |
| `org.ow2.asm:asm-commons:9.6` | BSD-3-Clause |
| `org.ow2.asm:asm-tree:9.6` | BSD-3-Clause |
| `org.ow2.asm:asm-util:9.6` | BSD-3-Clause |
| `org.slf4j:slf4j-api:1.7.30` | MIT License |
| `org.slf4j:slf4j-parent:1.7.30` | MIT License |
| `org.sonatype.oss:oss-parent:7` | **Unknown** |
| `org.sonatype.oss:oss-parent:9` | **Unknown** |
| `org.tensorflow:tensorflow-lite-metadata:0.1.0-rc2` | **Unknown** |

## Task 12 desktop shell dependency declarations

The standalone `desktop/Cargo.lock` pins the Tauri shell separately from the
Android/Core/CLI workspace. Direct additions: Tauri 2.12.3, tauri-build 2.7.2
and tauri-plugin-dialog 2.8.2 (each MIT OR Apache-2.0). Their source is
https://github.com/tauri-apps/tauri and https://github.com/tauri-apps/plugins-workspace.
The existing Core remains a path dependency; its licenses above still apply.
No Project NOMAD source is copied by this thin native shell.

The following additional or differently versioned registry records were read
from the exact desktop lockfile and cached normalized Cargo manifests. These
are package license declarations, not a claim that every platform-conditional
crate ships in the Windows executable or that all upstream notices were audited.
All records shared unchanged with the main lockfile retain their entries above.
Windows build/runtime evidence is recorded in RESULTS; Linux/macOS builds are
unverified. WebView2 is an externally installed Microsoft Evergreen runtime;
the M1 shell does not bundle or install that runtime. Third-party notices must
accompany any later distributable package; a lockfile declaration alone does
not replace those notices.

| Crate | Exact version | Declared license |
| --- | --- | --- |
| adler2 | 2.0.1 | 0BSD OR MIT OR Apache-2.0 |
| alloc-no-stdlib | 3.0.0 | BSD-3-Clause |
| alloc-stdlib | 0.3.0 | BSD-3-Clause |
| android_system_properties | 0.1.6 | MIT OR Apache-2.0 |
| atk | 0.18.2 | MIT |
| atk-sys | 0.18.2 | MIT |
| base64 | 0.21.7 | MIT OR Apache-2.0 |
| bit-set | 0.8.0 | Apache-2.0 OR MIT |
| bit-vec | 0.8.0 | Apache-2.0 OR MIT |
| bitflags | 1.3.2 | MIT/Apache-2.0 |
| brotli | 9.0.0 | BSD-3-Clause AND MIT |
| brotli-decompressor | 6.0.1 | BSD-3-Clause/MIT |
| bs58 | 0.5.1 | MIT/Apache-2.0 |
| bytemuck | 1.25.2 | Zlib OR Apache-2.0 OR MIT |
| cairo-rs | 0.18.5 | MIT |
| cairo-sys-rs | 0.18.2 | MIT |
| cargo-platform | 0.1.9 | MIT OR Apache-2.0 |
| cargo_metadata | 0.19.2 | MIT |
| cargo_toml | 1.0.1 | Apache-2.0 OR MIT |
| cesu8 | 1.1.0 | Apache-2.0/MIT |
| cfb | 0.14.0 | MIT |
| cfg-expr | 0.15.8 | MIT OR Apache-2.0 |
| chrono | 0.4.45 | MIT OR Apache-2.0 |
| combine | 4.6.8 | MIT |
| cookie | 0.18.2 | MIT OR Apache-2.0 |
| core-foundation | 0.10.1 | MIT OR Apache-2.0 |
| core-foundation-sys | 0.8.7 | MIT OR Apache-2.0 |
| core-graphics | 0.25.0 | MIT OR Apache-2.0 |
| core-graphics-types | 0.2.0 | MIT OR Apache-2.0 |
| cssparser | 0.37.0 | MPL-2.0 |
| cssparser-macros | 0.7.1 | MPL-2.0 |
| ctor | 1.0.13 | Apache-2.0 OR MIT |
| dbus | 0.9.12 | Apache-2.0/MIT |
| defmt | 1.1.1 | MIT OR Apache-2.0 |
| defmt-macros | 1.1.1 | MIT OR Apache-2.0 |
| defmt-parser | 1.0.0 | MIT OR Apache-2.0 |
| derive_more | 2.1.1 | MIT |
| derive_more-impl | 2.1.1 | MIT |
| dirs | 7.0.0 | MIT OR Apache-2.0 |
| dirs-sys | 0.5.0 | MIT OR Apache-2.0 |
| dlopen2 | 0.8.2 | MIT |
| dlopen2_derive | 0.4.3 | MIT |
| dom_query | 0.28.0 | MIT |
| dpi | 0.1.2 | Apache-2.0 AND MIT |
| dtoa | 1.0.11 | MIT OR Apache-2.0 |
| dtoa-short | 0.3.5 | MPL-2.0 |
| dunce | 1.0.5 | CC0-1.0 OR MIT-0 OR Apache-2.0 |
| dyn-clone | 1.0.20 | MIT OR Apache-2.0 |
| embed-resource | 3.0.12 | MIT |
| embed_plist | 1.2.2 | MIT OR Apache-2.0 |
| fdeflate | 0.3.7 | MIT OR Apache-2.0 |
| field-offset | 0.3.6 | MIT OR Apache-2.0 |
| foreign-types | 0.5.0 | MIT/Apache-2.0 |
| foreign-types-macros | 0.2.4 | MIT/Apache-2.0 |
| foreign-types-shared | 0.3.1 | MIT/Apache-2.0 |
| futures-executor | 0.3.34 | MIT OR Apache-2.0 |
| gdk | 0.18.2 | MIT |
| gdk-pixbuf | 0.18.5 | MIT |
| gdk-pixbuf-sys | 0.18.0 | MIT |
| gdk-sys | 0.18.2 | MIT |
| gdkwayland-sys | 0.18.2 | MIT |
| gdkx11 | 0.18.2 | MIT |
| gdkx11-sys | 0.18.2 | MIT |
| gio | 0.18.4 | MIT |
| gio-sys | 0.18.1 | MIT |
| glib | 0.18.5 | MIT |
| glib-macros | 0.18.5 | MIT |
| glib-sys | 0.18.1 | MIT |
| gobject-sys | 0.18.0 | MIT |
| gtk | 0.18.2 | MIT |
| gtk-sys | 0.18.2 | MIT |
| gtk3-macros | 0.18.2 | MIT |
| hashbrown | 0.12.3 | MIT OR Apache-2.0 |
| heck | 0.4.1 | MIT OR Apache-2.0 |
| hex | 0.4.3 | MIT OR Apache-2.0 |
| iana-time-zone | 0.1.65 | MIT OR Apache-2.0 |
| iana-time-zone-haiku | 0.1.2 | MIT OR Apache-2.0 |
| ico | 0.5.0 | MIT |
| indexmap | 1.9.3 | Apache-2.0 OR MIT |
| infer | 0.22.0 | MIT |
| javascriptcore-rs | 1.1.2 | MIT |
| javascriptcore-rs-sys | 1.1.1 | MIT |
| jiff | 0.2.38 | Unlicense OR MIT |
| jiff-core | 0.1.1 | Unlicense OR MIT |
| jiff-static | 0.2.38 | Unlicense OR MIT |
| jiff-tzdb | 0.1.9 | Unlicense OR MIT |
| jiff-tzdb-platform | 0.1.3 | Unlicense OR MIT |
| jni | 0.21.1 | MIT/Apache-2.0 |
| jni-sys | 0.3.1 | MIT OR Apache-2.0 |
| jni-sys | 0.4.1 | MIT OR Apache-2.0 |
| jni-sys-macros | 0.4.1 | MIT OR Apache-2.0 |
| json-patch | 4.2.0 | MIT/Apache-2.0 |
| jsonptr | 0.7.1 | MIT OR Apache-2.0 |
| keyboard-types | 0.8.3 | MIT OR Apache-2.0 |
| libappindicator | 0.9.0 | Apache-2.0 OR MIT |
| libappindicator-sys | 0.9.0 | Apache-2.0 OR MIT |
| libdbus-sys | 0.2.7 | Apache-2.0/MIT |
| libloading | 0.7.4 | ISC |
| libredox | 0.1.25 | MIT |
| memoffset | 0.9.1 | MIT |
| miniz_oxide | 0.8.9 | MIT OR Zlib OR Apache-2.0 |
| miniz_oxide | 0.9.1 | MIT OR Zlib OR Apache-2.0 |
| muda | 0.20.0 | Apache-2.0 OR MIT |
| ndk | 0.9.0 | MIT OR Apache-2.0 |
| ndk-context | 0.1.1 | MIT OR Apache-2.0 |
| ndk-sys | 0.6.0+11769913 | MIT OR Apache-2.0 |
| num_enum | 0.7.6 | BSD-3-Clause OR MIT OR Apache-2.0 |
| num_enum_derive | 0.7.6 | BSD-3-Clause OR MIT OR Apache-2.0 |
| objc2-app-kit | 0.3.2 | Zlib OR Apache-2.0 OR MIT |
| objc2-cloud-kit | 0.3.2 | Zlib OR Apache-2.0 OR MIT |
| objc2-core-data | 0.3.2 | Zlib OR Apache-2.0 OR MIT |
| objc2-core-foundation | 0.3.2 | Zlib OR Apache-2.0 OR MIT |
| objc2-core-graphics | 0.3.2 | Zlib OR Apache-2.0 OR MIT |
| objc2-core-image | 0.3.2 | Zlib OR Apache-2.0 OR MIT |
| objc2-core-location | 0.3.2 | Zlib OR Apache-2.0 OR MIT |
| objc2-core-text | 0.3.2 | Zlib OR Apache-2.0 OR MIT |
| objc2-exception-helper | 0.1.1 | Zlib OR Apache-2.0 OR MIT |
| objc2-foundation | 0.3.2 | MIT |
| objc2-io-surface | 0.3.2 | Zlib OR Apache-2.0 OR MIT |
| objc2-quartz-core | 0.3.2 | Zlib OR Apache-2.0 OR MIT |
| objc2-ui-kit | 0.3.2 | Zlib OR Apache-2.0 OR MIT |
| objc2-user-notifications | 0.3.2 | Zlib OR Apache-2.0 OR MIT |
| objc2-web-kit | 0.3.2 | Zlib OR Apache-2.0 OR MIT |
| option-ext | 0.2.0 | MPL-2.0 |
| pango | 0.18.3 | MIT |
| pango-sys | 0.18.0 | MIT |
| phf_macros | 0.13.1 | MIT |
| plist | 1.10.1 | MIT |
| png | 0.17.16 | MIT OR Apache-2.0 |
| png | 0.18.1 | MIT OR Apache-2.0 |
| portable-atomic | 1.15.0 | Apache-2.0 OR MIT |
| portable-atomic-util | 0.2.8 | Apache-2.0 OR MIT |
| proc-macro-crate | 1.3.1 | MIT OR Apache-2.0 |
| proc-macro-crate | 2.0.2 | MIT OR Apache-2.0 |
| proc-macro-error | 1.0.4 | MIT OR Apache-2.0 |
| proc-macro-error-attr | 1.0.4 | MIT OR Apache-2.0 |
| quick-xml | 0.42.0 | MIT |
| raw-window-handle | 0.6.2 | MIT OR Apache-2.0 OR Zlib |
| redox_users | 0.5.3 | MIT |
| ref-cast | 1.0.27 | MIT OR Apache-2.0 |
| ref-cast-impl | 1.0.27 | MIT OR Apache-2.0 |
| reqwest | 0.13.5 | MIT OR Apache-2.0 |
| rfd | 0.16.0 | MIT |
| schemars | 0.8.22 | MIT |
| schemars | 0.9.0 | MIT |
| schemars | 1.2.2 | MIT |
| schemars_derive | 0.8.22 | MIT |
| selectors | 0.38.0 | MPL-2.0 |
| serde-untagged | 0.1.9 | MIT OR Apache-2.0 |
| serde_derive_internals | 0.29.1 | MIT OR Apache-2.0 |
| serde_repr | 0.1.21 | MIT OR Apache-2.0 |
| serde_with | 3.24.0 | MIT OR Apache-2.0 |
| serde_with_macros | 3.24.0 | MIT OR Apache-2.0 |
| serialize-to-javascript | 0.1.2 | MIT OR Apache-2.0 |
| serialize-to-javascript-impl | 0.1.2 | MIT OR Apache-2.0 |
| servo_arc | 0.4.3 | MIT OR Apache-2.0 |
| simd-adler32 | 0.3.10 | MIT |
| softbuffer | 0.4.8 | MIT OR Apache-2.0 |
| soup3 | 0.5.0 | MIT |
| soup3-sys | 0.5.0 | MIT |
| swift-rs | 1.0.9 | MIT OR Apache-2.0 |
| syn | 1.0.109 | MIT OR Apache-2.0 |
| system-deps | 6.2.2 | MIT OR Apache-2.0 |
| tao | 0.37.1 | Apache-2.0 |
| tao-macros | 0.1.4 | MIT OR Apache-2.0 |
| target-lexicon | 0.12.16 | Apache-2.0 WITH LLVM-exception |
| tauri | 2.12.3 | Apache-2.0 OR MIT |
| tauri-build | 2.7.2 | Apache-2.0 OR MIT |
| tauri-codegen | 2.7.2 | Apache-2.0 OR MIT |
| tauri-macros | 2.7.2 | Apache-2.0 OR MIT |
| tauri-plugin | 2.7.1 | Apache-2.0 OR MIT |
| tauri-plugin-dialog | 2.8.2 | Apache-2.0 OR MIT |
| tauri-plugin-fs | 2.7.0 | Apache-2.0 OR MIT |
| tauri-runtime | 2.12.1 | Apache-2.0 OR MIT |
| tauri-runtime-wry | 2.12.1 | Apache-2.0 OR MIT |
| tauri-utils | 2.10.1 | Apache-2.0 OR MIT |
| tauri-winres | 0.3.6 | MIT |
| thiserror | 1.0.69 | MIT OR Apache-2.0 |
| thiserror-impl | 1.0.69 | MIT OR Apache-2.0 |
| toml | 0.8.2 | MIT OR Apache-2.0 |
| toml_datetime | 0.6.3 | MIT OR Apache-2.0 |
| toml_edit | 0.19.15 | MIT OR Apache-2.0 |
| toml_edit | 0.20.2 | MIT OR Apache-2.0 |
| tray-icon | 0.25.1 | MIT OR Apache-2.0 |
| unicode-segmentation | 1.13.3 | MIT OR Apache-2.0 |
| urlpattern | 0.6.0 | MIT |
| version-compare | 0.2.1 | MIT |
| vswhom | 0.1.0 | MIT |
| vswhom-sys | 0.1.3 | MIT |
| wasm-streams | 0.5.0 | MIT OR Apache-2.0 |
| webkit2gtk | 2.0.2 | MIT |
| webkit2gtk-sys | 2.0.2 | MIT |
| webview2-com | 0.39.1 | MIT |
| webview2-com-macros | 0.8.1 | MIT |
| webview2-com-sys | 0.39.1 | MIT |
| window-vibrancy | 0.8.1 | Apache-2.0 OR MIT |
| windows | 0.62.2 | MIT OR Apache-2.0 |
| windows-collections | 0.3.2 | MIT OR Apache-2.0 |
| windows-core | 0.62.2 | MIT OR Apache-2.0 |
| windows-future | 0.3.2 | MIT OR Apache-2.0 |
| windows-implement | 0.60.2 | MIT OR Apache-2.0 |
| windows-interface | 0.59.3 | MIT OR Apache-2.0 |
| windows-numerics | 0.3.1 | MIT OR Apache-2.0 |
| windows-result | 0.4.1 | MIT OR Apache-2.0 |
| windows-strings | 0.5.1 | MIT OR Apache-2.0 |
| windows-sys | 0.45.0 | MIT OR Apache-2.0 |
| windows-sys | 0.60.2 | MIT OR Apache-2.0 |
| windows-targets | 0.42.2 | MIT OR Apache-2.0 |
| windows-targets | 0.53.5 | MIT OR Apache-2.0 |
| windows-threading | 0.2.1 | MIT OR Apache-2.0 |
| windows-version | 0.1.7 | MIT OR Apache-2.0 |
| windows_aarch64_gnullvm | 0.42.2 | MIT OR Apache-2.0 |
| windows_aarch64_gnullvm | 0.53.1 | MIT OR Apache-2.0 |
| windows_aarch64_msvc | 0.42.2 | MIT OR Apache-2.0 |
| windows_aarch64_msvc | 0.53.1 | MIT OR Apache-2.0 |
| windows_i686_gnu | 0.42.2 | MIT OR Apache-2.0 |
| windows_i686_gnu | 0.53.1 | MIT OR Apache-2.0 |
| windows_i686_gnullvm | 0.53.1 | MIT OR Apache-2.0 |
| windows_i686_msvc | 0.42.2 | MIT OR Apache-2.0 |
| windows_i686_msvc | 0.53.1 | MIT OR Apache-2.0 |
| windows_x86_64_gnu | 0.42.2 | MIT OR Apache-2.0 |
| windows_x86_64_gnu | 0.53.1 | MIT OR Apache-2.0 |
| windows_x86_64_gnullvm | 0.42.2 | MIT OR Apache-2.0 |
| windows_x86_64_gnullvm | 0.53.1 | MIT OR Apache-2.0 |
| windows_x86_64_msvc | 0.42.2 | MIT OR Apache-2.0 |
| windows_x86_64_msvc | 0.53.1 | MIT OR Apache-2.0 |
| winnow | 0.5.40 | MIT |
| winreg | 0.56.0 | MIT |
| wry | 0.57.0 | Apache-2.0 OR MIT |
| x11 | 2.21.0 | MIT |
| x11-dl | 2.21.0 | MIT |
## Friendly sharing QR

`qrcode-generator` 1.4.4 — MIT; exact npm version locked in
`web-stub/package-lock.json`. Its verbatim copyright/MIT source notice is retained in the UI notices.

## Managed Files uploader

Actual Project NOMAD file-uploader UI adaptation retains its Apache-2.0 notice.
The following exact npm versions are locked; verbatim license texts are retained
in `web-stub/public/assets/react-ui-NOTICES.txt`. No remote provider plugins are
configured. npm audit still reports the existing MapLibre attribution advisory;
the existing attribution sanitization boundary is unchanged.

`@uppy/components` 1.2.0 omits LICENSE from npm. Its declared MIT license was
verified against the repository root at published npm gitHead
`9d87b9eab1f64c62f1708f5cec903def315c51e4`; exact root text is retained in
`third_party/uppy-components-1.2.0-LICENSE` (SHA256
`a0bceade78956c1954caa8dc9a8a665e78792ce94021baa15af72ffd6072b793`).
`mime-match`, `wildcard` and the existing `murmurhash-js` carry their full license
text in README.md; those files are retained verbatim too.

| Package | Version | License |
| --- | --- | --- |
| @transloadit/prettier-bytes | 0.3.5 | MIT |
| @uppy/components | 1.2.0 | MIT |
| @uppy/core | 5.2.0 | MIT |
| @uppy/dashboard | 5.1.0 | MIT |
| @uppy/locales | 5.1.0 | MIT |
| @uppy/provider-views | 5.2.2 | MIT |
| @uppy/react | 5.1.1 | MIT |
| @uppy/store-default | 5.0.0 | MIT |
| @uppy/thumbnail-generator | 5.1.0 | MIT |
| @uppy/utils | 7.2.0 | MIT |
| classnames | 2.5.1 | MIT |
| clsx | 2.1.1 | MIT |
| eventemitter3 | 5.0.4 | MIT |
| exifr | 7.1.3 | MIT |
| lodash | 4.18.1 | MIT |
| mime-match | 1.0.2 | ISC |
| namespace-emitter | 2.0.1 | MIT |
| nanoid | 5.1.16 | MIT |
| p-queue | 8.1.1 | MIT |
| p-timeout | 6.1.4 | MIT |
| preact | 10.29.8 | MIT |
| pretty-bytes | 6.1.1 | MIT |
| shallow-equal | 3.1.0 | MIT |
| use-sync-external-store | 1.7.0 | MIT |
| wildcard | 1.1.2 | MIT |

### Movies/Shows controls

Plyr is explicitly required by the approved Movies/Shows checklist. The installed
3.8.5 package retains MIT text in `LICENSE.md` (SHA256
`3092a30a4ded08145e3745173479f476ab47290334980cbeeb62f5d451d49850`).
Upstream now provides security updates only; this delivery uses its HTML5 player
with packaged local assets, not third-party embeds/CDNs. The existing notice
collector retains these exact installed package licenses and the lockfile pins
the complete dependency closure.

| Package | Version | License |
| --- | --- | --- |
| plyr | 3.8.5 | MIT |
| custom-event-polyfill | 1.0.7 | MIT |
| loadjs | 4.3.0 | MIT |
| rangetouch | 2.0.1 | MIT |
| url-polyfill | 1.1.14 | MIT |

### Standalone PDF reader (2026-11-13)

| Component | Exact version | License / retained source |
| --- | --- | --- |
| pdfjs-dist | 6.5.438 | Apache-2.0; npm lockfile pins the archive/integrity. `LICENSE` SHA256 `0d542e0c8804e39aa7f37eb00da5a762149dc682d7829451287e11b938e94594`. |
| Adobe CMaps | bundled in pdfjs-dist 6.5.438 | BSD-style; `cmaps/LICENSE` SHA256 `aa92ab5a472974865a96fd4a4e9c13bb41bf6fe1b309cb6b8da48bc9e19839a2`. |
| PDFium/Foxit standard font data | bundled in pdfjs-dist 6.5.438 | BSD-style; `standard_fonts/LICENSE_FOXIT` SHA256 `b578cdd2345840ada550bd12519533812320d5f1d21cf4c1c7e1b1b0a31c98b7`. |
| Liberation Sans font data | 1.07.4, unmodified in pdfjs-dist 6.5.438 | GPL-2.0 with Liberation font exceptions; separate font data, not relicensed as Apache-2.0. `standard_fonts/LICENSE_LIBERATION` SHA256 `d2c4d5b3e115a519cb58eb691aa64538397e2611f9ebe801392cf9667997e7dc`. |
| ICC profile data | bundled in pdfjs-dist 6.5.438 | CC0-1.0; `iccs/LICENSE` SHA256 `286e4fd7b447330b2c88e23890e3cd0a9d38cb398d4a59cb247f578ccbda3213`. |
| PDFium JBIG2 JavaScript decoder fallback | bundled in pdfjs-dist 6.5.438 | BSD-style plus PDF.js Apache notice; `wasm/LICENSE_JBIG2` SHA256 `9e66b7f1b934a28b37f3bc4dac97915de1674271e79a0a88182a18ed9731b4d1`. |
| OpenJPEG JavaScript decoder fallback | bundled in pdfjs-dist 6.5.438 | BSD-2-Clause plus PDF.js Apache notice; `wasm/LICENSE_OPENJPEG` SHA256 `a6af136f3e15038a666b61f376612a07d9a4e48cb7c01adbf3e33b3f14ab49b6`. |

The build retains verbatim package and data-asset notices at
`assets/react-ui-NOTICES.txt` and `assets/pdf/THIRD-PARTY-NOTICES.txt`; the
individual data licenses also remain beside the copied assets. Asset copies are
generated from the locked local package, never downloaded at app runtime.
Font provenance/version is verified in the tagged upstream
[standard-font README](https://github.com/mozilla/pdf.js/blob/v6.5.438/external/standard_fonts/README.md).
Keep the original font data/exception and upstream source/reinstallation rights
when distributing; do not replace Liberation with a differently licensed release
without updating its notice and generated metrics.

The lockfile also records optional Node-only `@napi-rs/canvas` 1.1.1 and
`@napi-rs/canvas-win32-x64-msvc` 1.1.1 (both declared MIT). They were installed
with npm lifecycle scripts disabled; neither native backend is imported or
bundled by the browser/app renderer. The browser notice collector explicitly
omits this Node-only optional branch; it does not waive a shipped component's
license. No QuickJS PDF scripting manager or WebAssembly engine is used/copied.

### EPUB/CBZ positioned archive readers (2026-11-13)

The following new lockfile entries are used by the bounded ZIP/XML reader or
its native build graph. The installed registry manifests and complete original
license texts were read; declarations below are not inferred from the design.
`flate2`1.1.10 was already recorded; its Rust backend is enabled explicitly.
No ZIP AES/decryption, other compression backends or on-disk extraction is enabled.

| Package | Exact version | Declared license |
| --- | --- | --- |
| zip | 9.5.1 | MIT |
| quick-xml | 0.42.0 | MIT |
| adler2 | 2.0.1 | 0BSD OR MIT OR Apache-2.0 |
| constant_time_eq | 0.6.1 | CC0-1.0 OR MIT-0 OR Apache-2.0 |
| core_detect | 1.0.0 | MIT OR Apache-2.0 |
| encoding_rs | 0.8.44 | (Apache-2.0 OR MIT) AND BSD-3-Clause |
| miniz_oxide | 0.9.1 | MIT OR Zlib OR Apache-2.0 |
| multiversion_no_op | 1.0.0 | Apache-2.0 OR MIT |
| simd-adler32 | 0.3.10 | MIT |
| simdutf8 | 0.1.5 | MIT OR Apache-2.0 |
| typed-path | 0.13.0 | MIT OR Apache-2.0 |
| zeroize_derive | 1.5.0 | Apache-2.0 OR MIT |

Verbatim notices for all12 packages are retained in
`android/licenses/document-archive-NOTICES.txt`,125,498B, SHA256
`19f418ae2fb2f10e853dd69ff82b16e36846ffa43ba97612081905b7faa1cfa2`.
Headers identify exact package/version, upstream crate source and original text
hash; the source license bytes remain unchanged. Android's existing license
asset task includes this file. The digital-reader Windows packaging must include
this new notice alongside the existing native license bundle before delivery.
The desktop lock now includes the same archive dependency versions, adding
9 packages and removing/updating no existing version. Its additional target-
specific `nix`0.31.3 entry is MIT and already inventoried above; it is not a
Windows runtime component. The existing libzim4.1.0 authoring environment also
creates the original307,912B embedded-document test fixture; no new runtime
dependency or copied Kiwix source was introduced for embedded document handling.

### Workshop geometry renderer (2026-11-13)

`three` **0.187.0**, MIT, Copyright ©2010–2026 three.js authors; exact npm
package pinned without lifecycle scripts. Verbatim `LICENSE` SHA256
`8b378ebe60e2fe500158cb0ac71cb5e8b7d92953c2abcc63a0eb90499653b5bc`.
Includes its WebGL renderer and original `OrbitControls` module, both used
locally without modification. Primary source:
https://github.com/mrdoob/three.js/tree/r187 . No additional npm dependency;
Three's archive/model loaders and vendored fflate code are not imported.
The notice collector retains the package's full MIT license in each UI bundle.

### Optional unified Chat broker and radio adapter (2026-11-14)

The optional Linux broker image uses Node22.23.2 bookworm-slim and the existing
SQLCipher helper built with Rust1.96.1 bookworm. The actual shipped Debian runtime
closure (88 installed packages with exact versions) is recorded in
`broker/licenses/linux-runtime/inventory.json`. All88 upstream distribution
copyright files and17 referenced common license texts are retained verbatim
under that directory, with byte counts and SHA256 values in the inventory.
`broker/scripts/record-linux-runtime.mjs` reproduces this inventory from the
image without networking or mounting service data. Node's full upstream license
above and the existing helper/native notices also ship in the image. Build-only
Rust/UI toolchains are separate stages and are not shipped runtime components.

The optional Windows broker package bundles the verified official Windows x64
Node.js22.23.2 runtime, SHA256
`0d0f5e39f9f3d9587bc19f73eab3c2c9c4903fd02d6dbf9c853dd81b3d95fad4`.
It matches the release's `win-x64/node.exe` entry at
https://nodejs.org/dist/v22.23.2/SHASUMS256.txt . The complete upstream Node MIT
and bundled-library notices are retained verbatim in
`broker/licenses/Node-v22.23.2-LICENSE.txt`,145485B, SHA256
`c738ae413cf561f174e34f6961f8ca458aae2369a73640dda6234c629b98bcc4`,
from https://raw.githubusercontent.com/nodejs/node/v22.23.2/LICENSE . Packaging
refuses a runtime or license that differs from `node-runtime.json`. This runtime
is optional server packaging and is not installed on the phone or required to
launch either native app.

The optional Node broker pins Aedes1.2.0 (MIT), MQTT.js5.16.1 (MIT), ws8.23.0
(MIT), and selfsigned5.5.0 (MIT). All74 resolved npm runtime packages, exact
versions and declared licenses are recorded in `broker/licenses/dependencies.md`;
verbatim upstream license/notice texts are retained in
`broker/licenses/THIRD-PARTY-NOTICES.txt`. `broker/package-lock.json` pins their
registry integrity. The closure contains only MIT, ISC, BSD-3-Clause, Apache-2.0
and0BSD declarations. No upstream application code or branding is copied.

The native history helper reuses rusqlite0.42.0 with the existing bundled
SQLCipher/vendored OpenSSL and serde_json1.0.154/zeroize1.9.1 already recorded.
It uses a separate Chat database and key; it never opens Notes.

`crates/meshtastic-fork` adapts the exact published meshtastic0.1.9 source,
upstream revision44016def9e3fefda307223ff759b4c8694bd000b, GPL-3.0.
Original notices and license are retained. Modified stream files are marked;
`TONALLI-CHANGES.md` and `scripts/vendor-meshtastic.mjs` identify/reproduce the
bounded queues, fail-closed decode overflow, bounded buffers and complete writes.
Generated protobufs, serial/BLE implementation and radio configuration behavior
are retained. No new native dependency version is selected by this patch.
STL/OBJ/3MF geometry parsing uses original Tonalli code plus existing bounded
ZIP and quick-xml dependencies already recorded above. No printing/material
conformance or external texture loading is claimed by this geometry preview.
