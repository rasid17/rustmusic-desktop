//! Size, position and maximized state of the main window, written as they change:
//! the app often ends without a clean exit (tray icon, `tauri dev`, crash), and the
//! window-state plugin only writes on exit.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use tauri::{AppHandle, Runtime};
use tauri_plugin_window_state::{AppHandleExt, StateFlags};

pub const FLAGS: StateFlags = StateFlags::SIZE
    .union(StateFlags::POSITION)
    .union(StateFlags::MAXIMIZED)
    .union(StateFlags::FULLSCREEN);

static GENERATION: AtomicU64 = AtomicU64::new(0);

/// Returns whether the process runs on a Wayland session.
pub fn is_wayland() -> bool {
    #[cfg(target_os = "linux")]
    {
        std::env::var_os("WAYLAND_DISPLAY").is_some()
    }
    #[cfg(not(target_os = "linux"))]
    {
        false
    }
}

/// Returns whether a Wayland tiling compositor (`niri`, `Hyprland`, `Sway`) is active.
pub fn is_tiling_wayland() -> bool {
    #[cfg(target_os = "linux")]
    {
        if !is_wayland() {
            return false;
        }
        if std::env::var_os("NIRI_SOCKET").is_some()
            || std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE").is_some()
            || std::env::var_os("SWAYSOCK").is_some()
        {
            return true;
        }
        if let Ok(desktop) = std::env::var("XDG_CURRENT_DESKTOP") {
            let d = desktop.to_ascii_lowercase();
            return d.contains("niri") || d.contains("hyprland") || d.contains("sway");
        }
        false
    }
    #[cfg(not(target_os = "linux"))]
    {
        false
    }
}

/// Effective window-state flags for the current platform and compositor.
pub fn flags() -> StateFlags {
    let mut f = FLAGS;
    if is_wayland() {
        f.remove(StateFlags::POSITION);
    }
    if is_tiling_wayland() {
        f.remove(StateFlags::MAXIMIZED);
    }
    f
}

pub fn save_now<R: Runtime>(app: &AppHandle<R>) {
    if let Err(e) = app.save_window_state(flags()) {
        log::warn!("État de la fenêtre non sauvegardé : {e}");
    }
}

/// Saves once the window has stopped moving for a moment.
pub fn schedule_save<R: Runtime>(app: AppHandle<R>) {
    let generation = GENERATION.fetch_add(1, Ordering::AcqRel) + 1;
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_millis(800)).await;
        if GENERATION.load(Ordering::Acquire) == generation {
            save_now(&app);
        }
    });
}
