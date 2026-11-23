# Optional Mesh terminal spike

This package defaults to no features and no radio runtime. The development
command uses the existing Meshtastic Rust crate for USB or BLE; TCP, HTTP/WSS,
history and a consumer Mesh UI are not implemented here. There is no mock radio
or simulated transport. Pure configuration checks are not hardware acceptance.

Build Windows: `rtk cargo build -p tonalli-mesh-gateway --features ble --bin tonalli-mesh-spike --release -j 1`.
The `ble` feature includes USB and BLE. A USB-only build uses `--features serial`.
Ordinary Tonalli builds do not depend on this package. Use `--help` for arguments.

Copy config.example.toml to private storage outside any source checkout. Supply
an explicit device selector, absolute private lock directory and enable the
configuration. Serial uses its explicitly configured baud/DTR/RTS settings. BLE
uses the chosen device's Bluetooth address; no device name/address is compiled
into the program. No automatic transport fallback or reconnect occurs.

Set `TONALLI_MESH_CONFIG` to that external file or pass `--config EXTERNAL_CONFIG`.
Every leaf setting can be overridden with `TONALLI_MESH__` plus uppercase dotted
components joined by `__`, such as `TONALLI_MESH__RADIO__TRANSPORT`. Firmware lists
are comma-separated in the environment; other types use ordinary decimal/bool
values. Unknown keys and invalid values produce a typed, value-redacted error.
The terminal configuration is intentionally smaller than the full product-design
example in docs/mesh-gateway; do not use that future-product schema for this spike.

Before **each** connection, disconnect existing radio clients deliberately. Then
run `tonalli-mesh-spike --config EXTERNAL_CONFIG --confirm-node-free`. Omitting
that handoff flag prevents opening a transport. Read-only inspection (no
`spike.text`) reports detected firmware, node count and enabled channel indexes,
then closes. It does not send text or change radio configuration.

To test transmission, externally set `radio.accepted_firmware` to the exact
detected firmware, select a discovered `radio.channel_index`, and supply
`spike.text`. `spike.show_test_text=true` deliberately displays test text in the
terminal; credentials and raw packets are never displayed. The send result means
queued to transport, not proven RF delivery. The receive step requires a decoded
text on the selected channel from a node other than the connected radio; local
SDK send echoes are discarded. A timeout remains an incomplete receive test.
Ctrl+C cancels; close is bounded. Firmware/channel changes and forced takeover
are unsupported. SDK internal queues remain unbounded: this time-limited spike
is not production multi-client load acceptance.

Android BLE uses a separately packaged, optional development host:
`scripts/build-mesh-android.ps1`, then
`android/gradle-local.ps1 -PmeshSpike=true :mesh-spike:assembleDebug`.
The normal APK remains untouched. The host has only a native permission/start/stop
launcher and connected-device foreground service, not a Mesh conversation UI.
Its private `files/mesh-spike.toml` selects BLE; `files/mesh-console.txt` is the
terminal-readable result. No radio starts merely by installing/opening it. The
start button requires the operator's per-run handoff. Restore the prior client's
connection after the spike. Radio hardware/version results must be recorded
separately from build results.

Upstream dependency and copied JNI source notices are in LICENSES.md and
third_party/btleplug-0.12.0. Keep the license texts with distributed binaries.
