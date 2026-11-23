use tonalli_core::tier::{tier_for, RamTier};

const GIB: u64 = 1 << 30;

#[test]
fn classifies_reported_ram_without_device_models() {
    let cases = [
        (0, RamTier::Lite),
        (56 * GIB / 10, RamTier::Lite),
        (74 * GIB / 10, RamTier::Standard),
        (112 * GIB / 10, RamTier::Standard),
        (149 * GIB / 10, RamTier::Reference),
        (24 * GIB, RamTier::Reference),
        (u64::MAX, RamTier::Reference),
    ];
    for (bytes, expected) in cases {
        assert_eq!(tier_for(bytes), expected, "RAM bytes: {bytes}");
    }
}

#[test]
fn standard_boundary_is_inclusive_at_six_and_a_half_gib() {
    let boundary = 13 * GIB / 2;
    assert_eq!(tier_for(boundary - 1), RamTier::Lite);
    assert_eq!(tier_for(boundary), RamTier::Standard);
    assert_eq!(tier_for(boundary + 1), RamTier::Standard);
}

#[test]
fn reference_boundary_is_inclusive_at_thirteen_gib() {
    let boundary = 13 * GIB;
    assert_eq!(tier_for(boundary - 1), RamTier::Standard);
    assert_eq!(tier_for(boundary), RamTier::Reference);
    assert_eq!(tier_for(boundary + 1), RamTier::Reference);
}

#[test]
fn non_ai_entry_points_do_not_read_ram_tiers() {
    // Resource policy belongs outside the pack/search/notes/server entry points.
    let ffi_operations = include_str!("../src/ffi.rs")
        .split_once("fn pack_boundary")
        .expect("the explicit RAM adapter precedes the operational FFI")
        .1;
    for (name, source) in [
        ("pack", include_str!("../src/pack.rs")),
        ("zim", include_str!("../src/zim.rs")),
        ("search", include_str!("../src/index.rs")),
        ("maps", include_str!("../src/tiles.rs")),
        ("notes", include_str!("../src/notes.rs")),
        ("server", include_str!("../src/server.rs")),
        ("LAN server", include_str!("../src/server/lan.rs")),
        (
            "LAN authentication",
            include_str!("../src/server/lan/http.rs"),
        ),
        (
            "LAN sessions",
            include_str!("../src/server/lan/sessions.rs"),
        ),
        ("operational FFI", ffi_operations),
        ("LAN operational FFI", include_str!("../src/ffi/lan.rs")),
    ] {
        for symbol in ["tier_for", "ram_tier", "RamTier"] {
            assert!(
                !source.contains(symbol),
                "{name} entry points must not read {symbol}"
            );
        }
    }
}

#[cfg(feature = "ffi")]
#[test]
fn exported_tiers_match_the_core_policy() {
    use tonalli_core::ffi;
    assert!(matches!(ffi::ram_tier(0), Ok(ffi::RamTier::Lite)));
    assert!(matches!(
        ffi::ram_tier(13 * GIB / 2),
        Ok(ffi::RamTier::Standard)
    ));
    assert!(matches!(
        ffi::ram_tier(13 * GIB),
        Ok(ffi::RamTier::Reference)
    ));
}
