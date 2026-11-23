//! Runtime policy for AI, cache sizes and indexing concurrency; never app access.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RamTier {
    Lite,
    Standard,
    Reference,
}

/// Classifies RAM reported in bytes; this never decides whether the app may launch.
pub fn tier_for(total_ram_bytes: u64) -> RamTier {
    const GIB: u64 = 1 << 30;
    match total_ram_bytes {
        bytes if bytes < 13 * GIB / 2 => RamTier::Lite,
        bytes if bytes < 13 * GIB => RamTier::Standard,
        _ => RamTier::Reference,
    }
}
