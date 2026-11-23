# Tonalli

A standalone offline knowledge and media app for Android and Windows, with a
shared React interface and Rust core. Licensed GPL-3.0-or-later.

## Source layout

- `crates/core`: ZIM reading/search, maps, encrypted Notes, content management,
  local sharing, media and optional AI.
- `android`: Android ARM64 shell and native integrations.
- `desktop`: Windows Tauri shell (separate Cargo workspace).
- `web-stub`: shared React GUI; the historical folder name is retained.
- `broker`: optional Node.js chat/MQTT service and Docker distribution.
- `third_party`, `LICENSES.md`, `NOTICE`: dependency licenses and provenance.

This is work-in-progress source, not a release or a claim of complete hardware
acceptance. Local development records, screenshots, machine-specific instructions
and the original commit history are intentionally excluded from this repository.
No downloaded content packs, application databases, account data, signing keys,
credentials or compiled personal-device builds are included.

## Development

Install the Rust toolchain pinned in `rust-toolchain.toml`, Node.js/npm, and the
platform prerequisites for Android or Tauri/Windows as appropriate. Android
requires a locally configured SDK/NDK and JDK; native SQLCipher builds require
the corresponding compiler and OpenSSL build prerequisites. Configure tool paths
for your environment; existing helper scripts are development conveniences.

Build the shared UI from `web-stub` with `npm ci` then `npm run build`.
Core checks use `cargo test --workspace`. Desktop uses its own manifest at
`desktop/Cargo.toml`. Android build/package helpers are in `android/`.
See [broker instructions](broker/README.md) for direct Node.js and Docker use.
Large optional test packs listed in `docs/spike/packs.toml` are not bundled.

Native apps do not require Docker or a NOMAD server. Network sharing is opt-in;
private Notes/settings are not shared. Notes use SQLCipher without a plaintext
fallback. Preserve all third-party notices when redistributing adapted code.

## Inspiration and acknowledgments

Tonalli is an independent project. It is inspired by, and builds on, these
free/open-source projects; thank you to their authors and communities:

- [Project NOMAD](https://github.com/Crosstalk-Solutions/project-nomad) (Crosstalk Solutions, Apache-2.0): the offline-first "knowledge server" concept. Portions of the UI are adapted from it; see `NOTICE` and `third_party/project-nomad/`.
- [Kiwix](https://www.kiwix.org/) / [openZIM](https://openzim.org/) and [libzim](https://github.com/openzim/libzim): the ZIM format used for offline Wikipedia and other content.
- [Protomaps](https://protomaps.com/) / PMTiles: single-file offline map archives.
- [Meshtastic](https://meshtastic.org/): off-grid LoRa mesh messaging.
- [llama.cpp](https://github.com/ggml-org/llama.cpp): local, optional AI inference.
- [Ventoy](https://www.ventoy.net/): multiboot USB concepts that informed the ISO tooling.
- [Tauri](https://tauri.app/), [React](https://react.dev/), [SQLCipher](https://www.zetetic.net/sqlcipher/) and the Rust ecosystem.

Full third-party licenses are in `LICENSES.md`, `NOTICE` and `third_party/`.
Mention here does not imply endorsement by any of these projects.
