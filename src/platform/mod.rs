#[cfg(no_std)]
pub use self::no_std::in_thread_mode as in_thread_mode;

#[cfg(no_std)]
pub use self::no_std::critical_section as critical_section;

#[cfg(not(no_std))]
pub use self::std::in_thread_mode as in_thread_mode;

#[cfg(not(no_std))]
pub use self::std::critical_section as critical_section;

#[cfg(target_arch = "arm")]
pub use cortex_m::asm::udf as abort;

#[cfg(not(no_std))]
pub use self::std::abort as abort;

#[cfg(target_arch = "arm")]
pub use self::arm::signal_event_ready as signal_event_ready;

#[cfg(not(no_std))]
pub use self::std::signal_event_ready as signal_event_ready;


#[cfg(target_arch = "arm")]
mod arm {
    #[inline]
    /// Prevent next `wait_for_interrupt` from sleeping, wake up other harts if needed.
    /// This particular implementation does nothing, since `wait_for_interrupt` never sleeps
    pub unsafe fn signal_event_ready() {
        asm::sev();
        //asm::nop();
    }

    #[inline]
    /// Wait for an interrupt or until notified by other hart via `signal_task_ready`
    /// This particular implementation does nothing
    pub unsafe fn wait_for_event() {
        asm::wfe();
        //asm::nop();
    }
}


#[cfg(no_std)]
pub mod no_std {
    pub fn in_thread_mode() -> bool {
        const SCB_ICSR: *const u32 = 0xE000_ED04 as *const u32;
        // NOTE(unsafe) single-instruction load with no side effects
        unsafe { SCB_ICSR.read_volatile() as u8 == 0 }
    }

    pub fn critical_section<F, R>(f: F) -> R
        where
            F: FnOnce() -> R,
    {
        cortex_m::interrupt::free(|cs| {
            f()
        })
    }
}

#[cfg(not(no_std))]
mod std {
    use std::sync::{Mutex, Once};
    use std::sync::atomic::AtomicBool;

    pub fn in_thread_mode() -> bool {
        true
    }

    pub fn abort() {
        panic!()
    }

    pub fn critical_section<F, R>(f: F) -> R
        where
            F: FnOnce() -> R,
    {
        static mut LOCK: Option<Mutex<AtomicBool>> = None;
        static LOCK_INIT: Once = Once::new();
        LOCK_INIT.call_once(|| {
            unsafe {
                LOCK.replace(Mutex::new(AtomicBool::new(false)));
            }
        });
        let _lock = unsafe { LOCK.as_ref().unwrap().lock().unwrap() };
        return f()
    }

    pub fn signal_event_ready() {
        // nop
    }
}
