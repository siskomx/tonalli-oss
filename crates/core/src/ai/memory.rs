use crate::pack::PackError;
use crate::tier::{tier_for, RamTier};

use super::ModelKind;

#[derive(Clone, Copy)]
pub(super) struct Snapshot {
    pub total: u64,
    pub available: u64,
}

#[derive(Clone, Copy)]
pub(super) struct Allocation {
    pub weights: u64,
    pub workspace: u64,
    pub total: u64,
}

/// The admission limits for one platform. `available` means different things per platform, so the
/// headroom kept back differs too.
#[derive(Clone, Copy)]
pub(super) struct Policy {
    /// Devices under 6.5 GiB total RAM are refused outright (Android and Windows).
    pub lite_refuses: bool,
    /// Memory kept free beyond the model and its workspace.
    pub headroom: u64,
    pub generation_workspace: u64,
    pub embedding_workspace: u64,
}

impl Policy {
    pub const STANDARD: Policy = Policy {
        lite_refuses: true,
        headroom: 1 << 30,
        generation_workspace: 1 << 30,
        embedding_workspace: 1 << 28,
    };
    /// iPhone: `available` is already the app's own headroom before iOS ends it, so only a small
    /// margin is kept, and a small model's 2048-token context needs far less than 1 GiB. Small phones
    /// are admitted by that real headroom instead of being refused for their total RAM.
    pub const IPHONE: Policy = Policy {
        lite_refuses: false,
        headroom: 1 << 29,
        generation_workspace: 3 << 27,
        embedding_workspace: 1 << 27,
    };
    pub const PLATFORM: Policy = if cfg!(target_os = "ios") {
        Policy::IPHONE
    } else {
        Policy::STANDARD
    };
}

pub(super) struct Budget {
    pub total: u64,
    pub uncommitted: u64,
    pub policy: Policy,
}

impl Default for Budget {
    fn default() -> Self {
        Self {
            total: 0,
            uncommitted: 0,
            policy: Policy::PLATFORM,
        }
    }
}

pub(super) fn snapshot() -> Result<Snapshot, PackError> {
    #[cfg(windows)]
    {
        // https://learn.microsoft.com/windows/win32/api/sysinfoapi/ns-sysinfoapi-memorystatusex
        #[repr(C)]
        #[derive(Default)]
        struct Status {
            length: u32,
            load: u32,
            total_physical: u64,
            available_physical: u64,
            total_page_file: u64,
            available_page_file: u64,
            total_virtual: u64,
            available_virtual: u64,
            available_extended_virtual: u64,
        }
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn GlobalMemoryStatusEx(status: *mut Status) -> i32;
        }
        let mut status = Status {
            length: u32::try_from(std::mem::size_of::<Status>()).map_err(|_| invalid_memory())?,
            ..Status::default()
        };
        // SAFETY: Status has the documented C layout, size and live output storage.
        if unsafe { GlobalMemoryStatusEx(&mut status) } == 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        validate(Snapshot {
            total: status.total_physical,
            available: status.available_physical,
        })
    }
    #[cfg(any(target_os = "android", target_os = "linux"))]
    {
        use std::io::Read;
        let mut text = String::new();
        std::fs::File::open("/proc/meminfo")?
            .take(65_537)
            .read_to_string(&mut text)?;
        parse_meminfo(&text)
    }
    #[cfg(target_os = "ios")]
    {
        use std::ffi::{c_char, c_int, c_void};
        unsafe extern "C" {
            // https://developer.apple.com/documentation/os/os_proc_available_memory (iOS 13+): the
            // headroom left before the system terminates this app, which is the real iPhone limit.
            fn os_proc_available_memory() -> usize;
            fn sysctlbyname(
                name: *const c_char,
                old: *mut c_void,
                old_len: *mut usize,
                new: *mut c_void,
                new_len: usize,
            ) -> c_int;
        }
        let mut total = 0u64;
        let mut length = std::mem::size_of::<u64>();
        // SAFETY: the name is NUL-terminated and the output is a live u64 whose size is passed.
        let status = unsafe {
            sysctlbyname(
                c"hw.memsize".as_ptr(),
                (&mut total as *mut u64).cast(),
                &mut length,
                std::ptr::null_mut(),
                0,
            )
        };
        if status != 0 || length != std::mem::size_of::<u64>() {
            return Err(std::io::Error::last_os_error().into());
        }
        // SAFETY: a plain query with no arguments.
        let headroom = unsafe { os_proc_available_memory() } as u64;
        validate(Snapshot {
            total,
            available: headroom.min(total),
        })
    }
    #[cfg(not(any(windows, target_os = "android", target_os = "linux", target_os = "ios")))]
    {
        Err(PackError::Mismatch(
            "AI memory accounting is unsupported on this platform".into(),
        ))
    }
}

fn invalid_memory() -> PackError {
    PackError::Corrupt("invalid runtime memory information".into())
}

fn validate(memory: Snapshot) -> Result<Snapshot, PackError> {
    if memory.total == 0 || memory.available > memory.total {
        return Err(invalid_memory());
    }
    Ok(memory)
}

#[cfg(any(test, target_os = "android", target_os = "linux"))]
fn parse_meminfo(text: &str) -> Result<Snapshot, PackError> {
    if text.len() > 65_536 {
        return Err(invalid_memory());
    }
    let (mut total, mut available) = (None, None);
    for line in text.lines() {
        let mut parts = line.split_ascii_whitespace();
        let target = match parts.next() {
            Some("MemTotal:") => &mut total,
            Some("MemAvailable:") => &mut available,
            _ => continue,
        };
        let value = parts
            .next()
            .ok_or_else(invalid_memory)?
            .parse::<u64>()
            .map_err(|_| invalid_memory())?
            .checked_mul(1024)
            .ok_or_else(invalid_memory)?;
        if parts.next() != Some("kB") || parts.next().is_some() || target.replace(value).is_some() {
            return Err(invalid_memory());
        }
    }
    validate(Snapshot {
        total: total.ok_or_else(invalid_memory)?,
        available: available.ok_or_else(invalid_memory)?,
    })
}

impl Budget {
    pub fn context_admission(&self, memory: Snapshot) -> Result<(), PackError> {
        let memory = validate(memory)?;
        if self.policy.lite_refuses && tier_for(memory.total) == RamTier::Lite {
            return Err(PackError::Mismatch(
                "AI is unavailable on the Lite RAM tier".into(),
            ));
        }
        if self.total > memory.total / 2
            || self.uncommitted > memory.available.saturating_sub(self.policy.headroom)
        {
            return Err(PackError::Mismatch(
                "AI context exceeds available memory budget".into(),
            ));
        }
        Ok(())
    }

    pub fn context_resident(&mut self, allocation: Allocation) -> Result<(), PackError> {
        self.uncommitted = self
            .uncommitted
            .checked_sub(allocation.workspace)
            .ok_or_else(|| PackError::Corrupt("AI context accounting inconsistent".into()))?;
        Ok(())
    }

    pub fn context_released(&mut self, allocation: Allocation) -> Result<(), PackError> {
        let pending = self
            .uncommitted
            .checked_add(allocation.workspace)
            .filter(|pending| *pending <= self.total)
            .ok_or_else(|| PackError::Corrupt("AI context accounting inconsistent".into()))?;
        self.uncommitted = pending;
        Ok(())
    }

    pub fn reserve(
        &mut self,
        memory: Snapshot,
        file_bytes: u64,
        kind: ModelKind,
    ) -> Result<Allocation, PackError> {
        let memory = validate(memory)?;
        if self.policy.lite_refuses && tier_for(memory.total) == RamTier::Lite {
            return Err(PackError::Mismatch(
                "AI is unavailable on the Lite RAM tier".into(),
            ));
        }
        // M1 estimates, not measured PSS: weights +12.5%, bounded future context /
        // workspace, <=50% total and the platform's headroom. Phone measurements still gate support.
        let overflow = || PackError::Mismatch("AI memory estimate overflow".into());
        let margin = file_bytes.div_ceil(8);
        let weights = file_bytes.checked_add(margin).ok_or_else(overflow)?;
        let workspace = match kind {
            ModelKind::Generation => self.policy.generation_workspace,
            ModelKind::Embedding => self.policy.embedding_workspace,
        };
        let allocation = Allocation {
            weights,
            workspace,
            total: weights.checked_add(workspace).ok_or_else(overflow)?,
        };
        let total = self
            .total
            .checked_add(allocation.total)
            .ok_or_else(overflow)?;
        let uncommitted = self
            .uncommitted
            .checked_add(allocation.total)
            .ok_or_else(overflow)?;
        if total > memory.total / 2
            || uncommitted > memory.available.saturating_sub(self.policy.headroom)
        {
            return Err(PackError::Mismatch(
                "AI model exceeds the memory budget".into(),
            ));
        }
        self.total = total;
        self.uncommitted = uncommitted;
        Ok(allocation)
    }

    pub fn resident(&mut self, allocation: Allocation) -> Result<(), PackError> {
        self.uncommitted = self
            .uncommitted
            .checked_sub(allocation.weights)
            .ok_or_else(|| PackError::Corrupt("AI memory accounting inconsistent".into()))?;
        Ok(())
    }

    pub fn release(&mut self, allocation: Allocation, resident: bool) -> Result<(), PackError> {
        let invalid = || PackError::Corrupt("AI memory accounting inconsistent".into());
        let total = self
            .total
            .checked_sub(allocation.total)
            .ok_or_else(invalid)?;
        let pending = if resident {
            allocation.workspace
        } else {
            allocation.total
        };
        let uncommitted = self.uncommitted.checked_sub(pending).ok_or_else(invalid)?;
        self.total = total;
        self.uncommitted = uncommitted;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GIB: u64 = 1 << 30;

    fn memory(available: u64) -> Snapshot {
        Snapshot {
            total: 16 * GIB,
            available,
        }
    }

    fn iphone() -> Budget {
        Budget {
            policy: Policy::IPHONE,
            ..Budget::default()
        }
    }

    #[test]
    fn iphone_admits_by_the_apps_headroom_so_a_4gb_phone_runs_only_a_small_model() {
        // iPhone 12: about 3.6 GiB total and roughly 2 GiB left before iOS ends the app.
        let phone = Snapshot {
            total: 3_900_000_000,
            available: 2 * GIB,
        };
        assert!(iphone()
            .reserve(phone, 450_000_000, ModelKind::Generation)
            .is_ok());
        assert!(iphone()
            .reserve(phone, 1_282_439_264, ModelKind::Generation)
            .is_err());
        // The Android and Windows rule still refuses this class of device outright.
        let mut standard = Budget {
            policy: Policy::STANDARD,
            ..Budget::default()
        };
        assert!(matches!(
            standard.reserve(phone, 450_000_000, ModelKind::Generation),
            Err(PackError::Mismatch(message)) if message.contains("Lite")
        ));
    }

    #[test]
    fn iphone_headroom_scales_the_choice_up_on_bigger_phones_and_never_past_it() {
        let six_gb = Snapshot {
            total: 6_000_000_000,
            available: 3 * GIB,
        };
        assert!(iphone()
            .reserve(six_gb, 1_282_439_264, ModelKind::Generation)
            .is_ok());
        assert!(iphone()
            .reserve(six_gb, 2_497_280_256, ModelKind::Generation)
            .is_err());
        // Nothing is admitted without real headroom, whatever the total RAM says.
        let squeezed = Snapshot {
            total: 8 * GIB,
            available: GIB / 4,
        };
        assert!(iphone()
            .reserve(squeezed, 450_000_000, ModelKind::Generation)
            .is_err());
    }

    #[test]
    fn lite_refuses_only_ai_admission() {
        let mut budget = Budget::default();
        let result = budget.reserve(
            Snapshot {
                total: 6 * GIB,
                available: 5 * GIB,
            },
            1024,
            ModelKind::Generation,
        );
        assert!(matches!(result, Err(PackError::Mismatch(message)) if message.contains("Lite")));
    }

    #[test]
    fn checked_estimate_and_available_headroom_are_enforced() {
        let mut budget = Budget::default();
        assert!(budget
            .reserve(memory(2 * GIB), GIB, ModelKind::Generation)
            .is_err());
        assert!(budget
            .reserve(
                Snapshot {
                    total: u64::MAX,
                    available: u64::MAX
                },
                u64::MAX,
                ModelKind::Generation
            )
            .is_err());
        assert_eq!(budget.total, 0);
        assert_eq!(budget.uncommitted, 0);
    }

    #[test]
    fn resident_generation_weights_are_not_charged_against_available_ram() {
        let mut budget = Budget::default();
        let generation = budget
            .reserve(memory(8 * GIB), 4 * GIB, ModelKind::Generation)
            .unwrap();
        budget.resident(generation).unwrap();
        assert_eq!(budget.uncommitted, generation.workspace);
        let embedding = budget
            .reserve(memory(3 * GIB), GIB / 16, ModelKind::Embedding)
            .unwrap();
        assert_eq!(embedding.weights, GIB / 16 + GIB / 128);
        budget.release(embedding, false).unwrap();
        budget.release(generation, true).unwrap();
        assert_eq!(budget.total, 0);
        assert_eq!(budget.uncommitted, 0);
    }

    #[test]
    fn parallel_pending_loads_cannot_overcommit_available_ram() {
        let mut budget = Budget::default();
        let pending = budget
            .reserve(memory(6 * GIB), 2 * GIB, ModelKind::Generation)
            .unwrap();
        assert!(budget
            .reserve(memory(6 * GIB), 2 * GIB, ModelKind::Generation)
            .is_err());
        budget.release(pending, false).unwrap();
        assert_eq!(budget.total, 0);
        assert_eq!(budget.uncommitted, 0);
    }

    #[test]
    fn aggregate_estimates_remain_bounded_by_half_actual_total() {
        let mut budget = Budget::default();
        let first = budget
            .reserve(memory(15 * GIB), 6 * GIB, ModelKind::Generation)
            .unwrap();
        budget.resident(first).unwrap();
        assert!(budget
            .reserve(memory(14 * GIB), GIB, ModelKind::Embedding)
            .is_err());
        budget.release(first, true).unwrap();
    }

    #[test]
    fn proc_meminfo_uses_checked_kib_values_and_rejects_ambiguous_data() {
        let parsed =
            parse_meminfo("MemTotal: 8388608 kB\nMemFree: 1 kB\nMemAvailable: 4194304 kB\n")
                .unwrap();
        assert_eq!(parsed.total, 8 * GIB);
        assert_eq!(parsed.available, 4 * GIB);
        for text in [
            "MemTotal: 1 kB\n",
            "MemTotal: x kB\nMemAvailable: 1 kB\n",
            "MemTotal: 4 kB\nMemAvailable: 2 MB\n",
            "MemTotal: 4 kB\nMemTotal: 4 kB\nMemAvailable: 2 kB\n",
            "MemTotal: 1 kB\nMemAvailable: 2 kB\n",
            "MemTotal: 18446744073709551615 kB\nMemAvailable: 0 kB\n",
        ] {
            assert!(parse_meminfo(text).is_err(), "{text}");
        }
    }

    #[test]
    fn runtime_snapshot_reports_real_nonzero_memory() {
        let memory = snapshot().unwrap();
        assert!(memory.total > 0);
        assert!(memory.available <= memory.total);
    }

    #[test]
    fn context_reuses_held_workspace_and_restores_it_after_cleanup() {
        let mut budget = Budget::default();
        let model = budget
            .reserve(memory(8 * GIB), GIB, ModelKind::Generation)
            .unwrap();
        budget.resident(model).unwrap();
        budget.context_admission(memory(2 * GIB)).unwrap();
        let total = budget.total;
        budget.context_resident(model).unwrap();
        assert_eq!(budget.uncommitted, 0);
        assert_eq!(budget.total, total);
        budget.context_released(model).unwrap();
        assert_eq!(budget.uncommitted, model.workspace);
        assert!(budget.context_admission(memory(GIB)).is_err());
        budget.release(model, true).unwrap();
        assert_eq!(budget.total, 0);
        assert_eq!(budget.uncommitted, 0);
    }
}
