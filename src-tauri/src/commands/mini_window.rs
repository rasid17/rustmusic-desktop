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

use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder};

pub const MINI_LABEL: &str = "mini";
/// Titre reconnu par les compositeurs pour l'image-dans-l'image (flottant par défaut dans niri).
const MINI_TITLE: &str = "Picture-in-Picture";
/// Taille initiale ; la hauteur réelle est mesurée puis appliquée par l'interface.
const MINI_W: f64 = 380.0;
const MINI_H: f64 = 250.0;

#[tauri::command]
pub fn open_mini_window(app: AppHandle, pinned: bool) -> Result<(), String> {
    if let Some(w) = app.get_webview_window(MINI_LABEL) {
        let _ = w.show();
        let _ = w.set_focus();
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
        .always_on_top(pinned)
        .skip_taskbar(true)
        // Créée cachée : l'interface l'affiche une fois peinte et dimensionnée, sans flash blanc.
        .visible(false)
        // Même fond que la fenêtre principale (tauri.conf.json) : pas d'écran blanc au départ.
        .background_color(tauri::window::Color(11, 13, 12, 255))
        .build()
        .map_err(|e| format!("création de la fenêtre mini : {e}"))?;
    log::info!("🪟  Fenêtre mini créée (épinglée : {pinned})");

    // Fermeture brutale (compositeur, Alt+F4) : la fenêtre principale, masquée,
    // doit le savoir pour réapparaître.
    let handle = app.clone();
    window.on_window_event(move |event| {
        if let tauri::WindowEvent::Destroyed = event {
            let _ = handle.emit("mini-window-closed", ());
        }
    });

    wire_pointer_events(&app, &window);
    Ok(())
}

#[tauri::command]
pub fn close_mini_window(app: AppHandle) {
    if let Some(w) = app.get_webview_window(MINI_LABEL) {
        let _ = w.destroy();
    }
}

/// WebKitGTK garde le survol quand le pointeur quitte la fenêtre ; GTK signale
/// l'entrée et la sortie, relayées au micro-lecteur (même montage que la
/// fenêtre principale dans `lib.rs`).
#[cfg(target_os = "linux")]
fn wire_pointer_events(app: &AppHandle, window: &tauri::WebviewWindow) {
    use gtk::gdk::{EventMask, NotifyType};
    use gtk::prelude::{WidgetExt, WidgetExtManual};
    let Ok(gtk_window) = window.gtk_window() else {
        return;
    };
    gtk_window.add_events(EventMask::ENTER_NOTIFY_MASK | EventMask::LEAVE_NOTIFY_MASK);
    let handle = app.clone();
    gtk_window.connect_enter_notify_event(move |_, event| {
        if event.detail() != NotifyType::Inferior {
            let _ = handle.emit("window-pointer", true);
        }
        gtk::glib::Propagation::Proceed
    });
    let handle = app.clone();
    gtk_window.connect_leave_notify_event(move |_, event| {
        if event.detail() != NotifyType::Inferior {
            let _ = handle.emit("window-pointer", false);
        }
        gtk::glib::Propagation::Proceed
    });
}

#[cfg(not(target_os = "linux"))]
fn wire_pointer_events(_app: &AppHandle, _window: &tauri::WebviewWindow) {}
