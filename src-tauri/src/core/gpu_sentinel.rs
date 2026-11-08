//! Crash sentinel for GPU-rendered boots (Linux).
//!
//! Some Linux stacks (Steam Deck / SteamOS, exotic Mesa builds) abort inside
//! WebKitGTK before the window ever paints (`EGL_BAD_PARAMETER`) or show a
//! blank white window. The user can never reach the render-mode setting in
//! that state, so we detect the failure ourselves :
//!
//! 1. Before booting the webview in GPU mode, we `arm()` a sentinel file.
//! 2. The frontend calls `notify_ui_ready` once real frames have been painted
//!    (double requestAnimationFrame), which `disarm()`s it. A graceful window
//!    close also disarms it (covers "user closed the app before JS mounted").
//! 3. If the sentinel is still armed at the next startup, the previous GPU
//!    boot never displayed anything → we persist `gpu_boot_failed` in the
//!    settings table and fall back to software rendering from now on.
//!
//! The sentinel lives next to the DB/logs in the app data dir. On Windows /
//! macOS the sentinel is never armed (webview stack is reliable there).

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

/// Settings key remembering that a GPU boot failed (read by the Auto mode).
pub const GPU_BOOT_FAILED_KEY: &str = "gpu_boot_failed";

/// Number of consecutive unconfirmed GPU boots required before declaring a
/// persistent GPU boot failure (prevents a single `pkill` or session logout
/// during startup from permanently locking the app into software rendering).
pub const MAX_UNCONFIRMED_BOOTS: u32 = 2;

/// Whether the current boot went down the GPU path (set at startup, read by
/// `notify_ui_ready` to know if a successful boot should clear the flag).
static BOOTED_GPU: AtomicBool = AtomicBool::new(false);

/// Set when WebKit's render process died mid-session (`web-process-terminated`
/// signal). Blocks the graceful-close disarm : a user closing the resulting
/// white window with Alt+F4 must NOT erase the recorded failure.
static WEB_PROCESS_CRASHED: AtomicBool = AtomicBool::new(false);

fn sentinel_path() -> PathBuf {
    let mut p = dirs::data_dir().unwrap_or_else(|| PathBuf::from("."));
    p.push("com.larevuegeek.rustmusic");
    p.push("gpu_boot_attempt");
    p
}

/// Returns the number of consecutive unconfirmed GPU boot attempts recorded in
/// the sentinel file (`0` if disarmed, `MAX_UNCONFIRMED_BOOTS` if an explicit
/// WebKit crash was recorded).
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn previous_attempts() -> u32 {
    let Ok(raw) = std::fs::read_to_string(sentinel_path()) else {
        return 0;
    };
    let trimmed = raw.trim();
    if trimmed == "crash" {
        return MAX_UNCONFIRMED_BOOTS;
    }
    trimmed.parse::<u32>().unwrap_or(1).max(1)
}

/// True if previous GPU boots repeatedly failed to display the UI or crashed.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn is_armed() -> bool {
    previous_attempts() >= MAX_UNCONFIRMED_BOOTS
}

/// Write the sentinel file with the current attempt counter right before a GPU boot.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn arm_with_count(attempt: u32) {
    let path = sentinel_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Err(e) = std::fs::write(&path, attempt.max(1).to_string()) {
        log::warn!("GPU sentinel : cannot arm {} : {}", path.display(), e);
    }
}

/// Write the sentinel file right before a GPU boot attempt.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn arm() {
    arm_with_count(1);
}

/// Record an explicit WebKit web-process crash so the next boot immediately
/// falls back to software rendering without waiting for a second attempt.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn arm_crashed() {
    let path = sentinel_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Err(e) = std::fs::write(&path, b"crash") {
        log::warn!("GPU sentinel : cannot arm crash {} : {}", path.display(), e);
    }
}

/// Remove the sentinel — the boot is confirmed OK (UI painted or clean exit).
pub fn disarm() {
    let _ = std::fs::remove_file(sentinel_path());
}

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn set_booted_gpu(v: bool) {
    BOOTED_GPU.store(v, Ordering::Relaxed);
}

pub fn booted_gpu() -> bool {
    BOOTED_GPU.load(Ordering::Relaxed)
}

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn mark_web_process_crashed() {
    WEB_PROCESS_CRASHED.store(true, Ordering::Relaxed);
}

pub fn web_process_crashed() -> bool {
    WEB_PROCESS_CRASHED.load(Ordering::Relaxed)
}
