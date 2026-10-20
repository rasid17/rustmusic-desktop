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

pub fn save_now<R: Runtime>(app: &AppHandle<R>) {
    if let Err(e) = app.save_window_state(FLAGS) {
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
