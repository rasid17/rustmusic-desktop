//! Fenêtre du mini-lecteur.
//!
//! Une vraie fenêtre distincte plutôt que la fenêtre principale rétrécie :
//! les compositeurs Wayland peuvent alors la traiter en image-dans-l'image
//! (niri fait flotter d'office le titre « Picture-in-Picture »), et la
//! fenêtre principale garde sa taille, ses règles et son comportement.
//!
//! La fenêtre principale se masque tant que le mini-lecteur est affiché ;
//! elle réapparaît dès que celui-ci se ferme, y compris brutalement
//! (compositeur, Alt+F4) grâce à l'événement `mini-window-closed`.

use std::sync::atomic::{AtomicBool, Ordering};

use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder};

pub const MINI_LABEL: &str = "mini";
static ALLOW_EXIT: AtomicBool = AtomicBool::new(false);

pub fn allow_exit() {
    ALLOW_EXIT.store(true, Ordering::SeqCst);
}

pub fn is_exit_allowed() -> bool {
    ALLOW_EXIT.load(Ordering::SeqCst)
}
/// Titre reconnu par les compositeurs pour l'image-dans-l'image (flottant par défaut dans niri).
const MINI_TITLE: &str = "Picture-in-Picture";
/// Taille initiale ; la hauteur réelle est mesurée puis appliquée par l'interface.
const MINI_W: f64 = 380.0;
const MINI_H: f64 = 250.0;

#[tauri::command]
pub fn open_mini_window(app: AppHandle, pinned: bool) -> Result<(), String> {
    if let Some(w) = app.get_webview_window(MINI_LABEL) {
        let _ = w.set_always_on_top(pinned);
        let _ = app.emit_to(MINI_LABEL, "mini-reopen", ());
        return Ok(());
    }

    let window = WebviewWindowBuilder::new(&app, MINI_LABEL, WebviewUrl::App("index.html".into()))
        .title(MINI_TITLE)
        .inner_size(MINI_W, MINI_H)
        // Redimensionnable : GTK3 ignore toute taille demandée sur une fenêtre
        // créée non redimensionnable ; le contenu (panneau, micro) impose sa
        // taille au fil de la session.
        .resizable(true)
        .decorations(false)
        .transparent(true)
        .shadow(false)
        .always_on_top(pinned)
        .skip_taskbar(true)
        // Créée cachée : l'interface l'affiche une fois peinte et dimensionnée, sans flash blanc.
        .visible(false)
        // Fond transparent RGBA pour permettre les silhouettes Pill (pochette ronde débordante) et Sleeve.
        .background_color(tauri::window::Color(0, 0, 0, 0))
        .build()
        .map_err(|e| format!("création de la fenêtre mini : {e}"))?;
    log::info!("🪟  Fenêtre mini créée (épinglée : {pinned})");

    // Fermeture depuis le compositeur (Mod+Q sous niri/Hyprland, Alt+F4) :
    // on masque la fenêtre mini pour la garder chaude en mémoire, et on
    // prévient la fenêtre principale pour qu'elle réapparaisse.
    let handle = app.clone();
    let win_for_event = window.clone();
    window.on_window_event(move |event| match event {
        tauri::WindowEvent::CloseRequested { api, .. } => {
            if ALLOW_EXIT.load(Ordering::SeqCst) {
                return;
            }
            api.prevent_close();
            let _ = win_for_event.hide();
            let _ = handle.emit_to(MINI_LABEL, "mini-suspend", ());
            let _ = handle.emit("mini-window-closed", ());
        }
        tauri::WindowEvent::Destroyed => {
            let _ = handle.emit("mini-window-closed", ());
        }
        _ => {}
    });

    wire_pointer_events(&app, &window);
    Ok(())
}

#[tauri::command]
pub fn close_mini_window(app: AppHandle) {
    if let Some(w) = app.get_webview_window(MINI_LABEL) {
        let _ = app.emit_to(MINI_LABEL, "mini-suspend", ());
        let _ = w.hide();
    }
}

/// WebKitGTK garde le survol quand le pointeur quitte la fenêtre ; GTK signale
/// l'entrée et la sortie, relayées au micro-lecteur (même montage que la
/// fenêtre principale dans `lib.rs`).
#[cfg(target_os = "linux")]
fn wire_pointer_events(app: &AppHandle, window: &tauri::WebviewWindow) {
    use gtk::gdk::{EventMask, NotifyType};
    use gtk::prelude::{GtkWindowExt, WidgetExt, WidgetExtManual};

    fn attach_crossing_signals(widget: &impl WidgetExt, app: &AppHandle) {
        widget.add_events(EventMask::ENTER_NOTIFY_MASK | EventMask::LEAVE_NOTIFY_MASK);
        if let Some(gdk_win) = widget.window() {
            gdk_win.set_events(gdk_win.events() | EventMask::ENTER_NOTIFY_MASK | EventMask::LEAVE_NOTIFY_MASK);
        }
        let handle = app.clone();
        widget.connect_enter_notify_event(move |_, event| {
            if event.detail() != NotifyType::Inferior {
                let _ = handle.emit("window-pointer", true);
            }
            gtk::glib::Propagation::Proceed
        });
        let handle = app.clone();
        widget.connect_leave_notify_event(move |_, event| {
            if event.detail() != NotifyType::Inferior {
                let _ = handle.emit("window-pointer", false);
            }
            gtk::glib::Propagation::Proceed
        });
    }

    if let Ok(gtk_window) = window.gtk_window() {
        gtk_window.set_role("picture-in-picture");
        attach_crossing_signals(&gtk_window, app);
        let handle = app.clone();
        gtk_window.connect_focus_out_event(move |_, _| {
            let _ = handle.emit("window-pointer", false);
            gtk::glib::Propagation::Proceed
        });
    }

    let handle = app.clone();
    let _ = window.with_webview(move |platform_webview| {
        let wv = platform_webview.inner();
        attach_crossing_signals(&wv, &handle);
        let handle_realize = handle.clone();
        wv.connect_realize(move |w| {
            if let Some(gdk_win) = w.window() {
                gdk_win.set_events(gdk_win.events() | EventMask::ENTER_NOTIFY_MASK | EventMask::LEAVE_NOTIFY_MASK);
            }
            let _ = handle_realize.emit("window-pointer", false);
        });
    });
}

#[cfg(not(target_os = "linux"))]
fn wire_pointer_events(_app: &AppHandle, _window: &tauri::WebviewWindow) {}
