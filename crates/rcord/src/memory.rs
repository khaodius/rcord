//! Hands freed heap back to the system. glibc keeps one heap arena per busy thread and
//! holds on to what is freed in each, so after the big READY parse or a long session a
//! large part of the resident size is memory the app is no longer using.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

static TRIM_QUEUED: AtomicBool = AtomicBool::new(false);

/// Caps the number of heap arenas and starts a thread that trims the heap once a minute.
/// Call before other threads start.
pub fn init() {
    imp::limit_arenas();
    std::thread::Builder::new()
        .name("rcord-trim".into())
        .spawn(|| {
            loop {
                std::thread::sleep(Duration::from_secs(60));
                imp::trim();
            }
        })
        .ok();
}

/// Trims the heap on a background thread a moment from now, once the work that freed
/// the memory has finished.
pub fn trim_soon() {
    if TRIM_QUEUED.swap(true, Ordering::AcqRel) {
        return;
    }
    std::thread::Builder::new()
        .name("rcord-trim-now".into())
        .spawn(|| {
            std::thread::sleep(Duration::from_secs(2));
            TRIM_QUEUED.store(false, Ordering::Release);
            imp::trim();
        })
        .ok();
}

#[cfg(all(target_os = "linux", target_env = "gnu"))]
#[allow(unsafe_code)]
mod imp {
    pub fn limit_arenas() {
        // SAFETY: mallopt only changes allocator tuning and is called before other threads.
        unsafe {
            libc::mallopt(libc::M_ARENA_MAX, 2);
        }
    }

    pub fn trim() {
        // SAFETY: malloc_trim is thread-safe and only returns free pages to the system.
        unsafe {
            libc::malloc_trim(0);
        }
    }
}

#[cfg(not(all(target_os = "linux", target_env = "gnu")))]
mod imp {
    pub fn limit_arenas() {}
    pub fn trim() {}
}
