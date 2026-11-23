# Tonalli adaptation of meshtastic 0.1.9

BLE discovery also stops its scan after collecting results; this is reproduced
by the same script. Gateway GATT release is handled in Tonalli's own adapter.

Upstream test-only modules are omitted by `scripts/omit-meshtastic-test-fixtures.mjs`
to honor the user's real-device/no-added-mocks delivery direction. Runtime APIs
and protocol code remain; the exact original crate and revision identify the
omitted upstream fixtures. No simulated radio transport or mock CI is added.

Exact crates.io package, GPL-3.0; original LICENSE retained. Only stream queue/framing changes: outbound16, read32, decoded128 entries; async backpressure on byte/write queues; decoded overflow ends processing; framing66560B and log1024B limits; write_all prevents partial writes. No radio/protobuf/GATT replacement. Reproduce with scripts/vendor-meshtastic.mjs. Hardware acceptance remains required.

iOS: `available_serial_ports` returns no ports there (src/utils_internal.rs), because iOS has no USB host and the serialport lister needs IOKit.
