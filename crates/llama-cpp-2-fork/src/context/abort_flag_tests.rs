//! Tonalli additions: compile contract and pure callback ownership regressions.

use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Weak};

use super::{abort_flag_callback, AbortFlagSlot, LlamaContext};

fn invoke(callback: llama_cpp_sys_2::ggml_abort_callback, data: *mut c_void) -> bool {
    // SAFETY: Tests invoke only our callback with null or a live AtomicBool pointer.
    unsafe { callback.expect("installed abort callback")(data) }
}

#[test]
fn public_context_accepts_an_owned_sendable_abort_flag() {
    fn install(context: &mut LlamaContext<'_>, flag: Arc<AtomicBool>) {
        context.set_abort_flag(flag);
    }
    let _compile_contract = install;
}

#[test]
fn null_callback_data_aborts_without_dereferencing() {
    // SAFETY: Null must be handled before dereferencing callback data.
    assert!(unsafe { abort_flag_callback(std::ptr::null_mut()) });
}

#[test]
fn callback_observes_the_shared_flag_without_consuming_its_owner() {
    let flag = Arc::new(AtomicBool::new(false));
    let data = Arc::as_ptr(&flag).cast_mut().cast::<c_void>();
    // SAFETY: The Arc keeps this exact AtomicBool alive throughout both calls.
    assert!(!unsafe { abort_flag_callback(data) });
    flag.store(true, Ordering::Release);
    // SAFETY: The Arc remains live and this callback does not consume it.
    assert!(unsafe { abort_flag_callback(data) });
    assert_eq!(Arc::strong_count(&flag), 1);
}

#[test]
fn replacement_installs_the_new_pointer_before_releasing_the_old_flag() {
    let first = Arc::new(AtomicBool::new(false));
    let first_weak = Arc::downgrade(&first);
    let second = Arc::new(AtomicBool::new(true));
    let second_weak = Arc::downgrade(&second);
    let mut slot = AbortFlagSlot::default();

    slot.replace(first, |callback, data| {
        assert!(!invoke(callback, data));
    });
    assert_eq!(first_weak.strong_count(), 1);
    slot.replace(second, |callback, data| {
        assert_eq!(
            first_weak.strong_count(),
            1,
            "old callback may still be active"
        );
        assert!(
            second_weak.strong_count() >= 1,
            "new callback data must be owned"
        );
        assert!(
            invoke(callback, data),
            "the installed pointer must select the new flag"
        );
    });
    assert!(first_weak.upgrade().is_none());
    assert_eq!(second_weak.strong_count(), 1);
    drop(slot);
    assert!(second_weak.upgrade().is_none());
}

#[test]
fn slot_keeps_the_flag_alive_during_its_owners_teardown_body() {
    struct Owner {
        _slot: AbortFlagSlot,
        weak: Weak<AtomicBool>,
        teardown_saw_live_flag: Arc<AtomicBool>,
    }
    impl Drop for Owner {
        fn drop(&mut self) {
            self.teardown_saw_live_flag
                .store(self.weak.upgrade().is_some(), Ordering::Release);
        }
    }
    let flag = Arc::new(AtomicBool::new(false));
    let weak = Arc::downgrade(&flag);
    let observed = Arc::new(AtomicBool::new(false));
    let mut slot = AbortFlagSlot::default();
    slot.replace(flag, |callback, data| {
        assert!(!invoke(callback, data));
    });
    drop(Owner {
        _slot: slot,
        weak: weak.clone(),
        teardown_saw_live_flag: Arc::clone(&observed),
    });
    assert!(observed.load(Ordering::Acquire));
    assert!(weak.upgrade().is_none());
}
