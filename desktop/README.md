# Desktop M1 shell

This Windows-first Tauri shell runs `tonalli-core` in the same process and opens
the existing shared React UI in WebView2. The UI is supplied by `--web`; it is
not copied or embedded again. WebView2 must already be installed.

From the repository root, with the existing native toolchain and at least 5 GiB
available RAM and no concurrent build or emulator:

```powershell
rtk proxy powershell -NoProfile -ExecutionPolicy Bypass -File scripts/with-toolchain.ps1 Native rtk cargo build --manifest-path desktop/Cargo.toml --locked --offline -j 1 --target-dir target
rtk proxy target/debug/tonalli-desktop.exe --web web-stub/dist --data-dir target/desktop-m1/data --profile-dir target/desktop-m1/webview2 --pack probe=docs/spike/packs/wikipedia_en_knots_maxi_2026-10.zim --iso-inner /fixture.zim
```

The standalone Cargo workspace has its own lockfile so Tauri's platform graph
does not alter the existing CLI/Android dependency graph. Use the root target
directory to reuse compiled dependencies. `--release` uses the approved fat-LTO,
one-codegen-unit, unwind, stripped profile; debug runs are functional checks.
The built shared UI must exist before launching; the shell never starts a Node
server or another Core process. `branding.json` supplies the product/window/tray
name. The small generic orange icon was authored for this shell.

`--pack ID=PATH` can repeat for `.zim` or `.pmtiles` files. Map country IDs are
`mx`, `us` and `ca`. An optional `--title-index ID=PATH` attaches Core's matching
verified positioned-read title table. All manually supplied packs/tables are
trusted M1 inputs; the existing catalog/install lifecycle remains Core work.
`--data-dir` and `--profile-dir` must be private app-owned directories outside
the served web root. The profile defaults to `<data-dir>/webview2` and never uses
the user's Edge profile. Existing data is retained on exit.

Use **Open ISO…** in the native menu or tray, or press **Ctrl+O**. For M1,
`--iso-inner` must give the exact case-sensitive contained ZIM path. The example
matches the checked `docs/spike/packs/small-zim-20261111.iso`; the shell never
guesses a first ZIM. The native picker selects only the outer image. One worker
opens it read-only using `PreadSource`/`IsoImage`, verifies the embedded ZIM,
then registers it as `iso` and opens that pack in the shared library. Failed
opens preserve prior packs. A second publication of `iso` is refused for M1;
restart the process to choose a different image. No mounting or extraction is
performed.

**Show window** shows/focuses the current screen. **Quit** ends
the app and its private loopback server. Closing the last window also exits.
The shell has no JavaScript invoke commands or remote capabilities. It allows
navigation only to its exact owned `http://127.0.0.1:<port>` origin and denies
new windows; the Core retains its authentication, CSP and private Notes policy.
No LAN listener starts here.

Checked release runtime: native menu opens the ISO picker; selecting the pinned
image opens its library/article in place. All three map samples render and native
menu Quit exits cleanly. Automation did not establish keyboard-shortcut or tray
interaction acceptance (injected Ctrl+Q did not exit); use native menu clicks for
this checkpoint. Minimized-window restoration remains unverified. The Notes
screen is checked locked; its shared vault-creation flow is still unfinished.

For an owned diagnostic run only, an external harness may set process-local
`WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=0` and choose a
fresh `--profile-dir`. Debug flags are never enabled by the shell. The local
`desktop_window_created_ms` stderr diagnostic measures native window
construction, not first paint or content readiness. Cold OS/cache conditions,
shared UI render timings and whole-process-tree RAM are measured separately.

Focused checks:

```powershell
rtk proxy powershell -NoProfile -ExecutionPolicy Bypass -File scripts/with-toolchain.ps1 Native rtk cargo test --manifest-path desktop/Cargo.toml --no-default-features --lib --locked --offline -j 1 --target-dir target
rtk proxy powershell -NoProfile -ExecutionPolicy Bypass -File scripts/with-toolchain.ps1 Native rtk cargo test --manifest-path desktop/Cargo.toml --bin tonalli-desktop --locked --offline -j 1 --target-dir target
```

This is an M1 runnable executable, not an installer or a complete v1 app.
Distribution must retain the project's license inventory and the actual license
texts for the dependencies shipped with the shell/UI.
