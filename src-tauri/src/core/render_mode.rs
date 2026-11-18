//! User override for the WebKit / GTK rendering mode.
//!
//! By default (`Auto`), we detect a VM at startup and apply software
//! rendering env vars only there. But some users may want to force one
//! mode or the other :
//!   - `ForceGpu`      → trust the system, don't touch env vars (best perf
//!                       on native ; risky on KDE Wayland / weird stacks)
//!   - `ForceSoftware` → always apply the software-rendering env vars
//!                       (works everywhere ; less smooth animations)
//!
//! IMPORTANT : env vars are read by WebKitGTK at process init, so changing
//! this setting requires an app restart to take effect.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RenderMode {
    /// Auto-detection : software rendering on VMs, GPU on native.
    Auto,
    /// Always use the GPU (no env vars set).
    ForceGpu,
    /// Always force software rendering (set all the WebKit/GDK env vars).
    ForceSoftware,
}

impl RenderMode {
    /// Parse the value persisted in the `settings` table (defaults to `Auto`).
    pub fn parse_or_auto(value: Option<&str>) -> Self {
        match value.map(str::to_ascii_lowercase).as_deref() {
            Some("force-gpu") | Some("gpu") => Self::ForceGpu,
            Some("force-software") | Some("software") => Self::ForceSoftware,
            _ => Self::Auto,
        }
    }

    /// Read the `RUSTMUSIC_RENDER` env override, if set.
    ///
    /// Escape hatch for users whose app crashes before they can reach the
    /// settings UI (e.g. `RUSTMUSIC_RENDER=software ./RustMusic.AppImage`),
    /// and for re-testing GPU on a machine remembered as failing
    /// (`RUSTMUSIC_RENDER=gpu`). Takes precedence over the persisted setting;
    /// `auto` re-runs the detection while ignoring the DB value.
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    pub fn from_env() -> Option<Self> {
        let raw = std::env::var("RUSTMUSIC_RENDER").ok()?;
        match raw.trim().to_ascii_lowercase().as_str() {
            "gpu" | "force-gpu" => Some(Self::ForceGpu),
            "software" | "force-software" | "cpu" => Some(Self::ForceSoftware),
            "auto" => Some(Self::Auto),
            "" => None,
            other => {
                log::warn!("RUSTMUSIC_RENDER : unknown value '{}' (expected gpu | software | auto), ignoring", other);
                None
            }
        }
    }

    /// Stable string form for persistence.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::ForceGpu => "force-gpu",
            Self::ForceSoftware => "force-software",
        }
    }
}

/// Réglage du moteur DMA-BUF de WebKitGTK (Linux), clé de la table `settings`.
/// `auto` le laisse actif sauf avec le pilote propriétaire NVIDIA ; `on` et
/// `off` imposent. Lu au démarrage, avant WebKit : un changement attend le
/// redémarrage.
pub const DMABUF_KEY: &str = "dmabuf_mode";

/// La valeur enregistrée, ramenée à `auto`, `on` ou `off`.
pub fn dmabuf_mode(value: Option<&str>) -> &'static str {
    match value.map(str::trim) {
        Some("on") => "on",
        Some("off") => "off",
        _ => "auto",
    }
}

static DMABUF_ACTIF: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Fixé au démarrage : DMA-BUF est-il actif pour cette session ?
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn set_dmabuf_active(actif: bool) {
    DMABUF_ACTIF.store(actif, std::sync::atomic::Ordering::Relaxed);
}

pub fn dmabuf_active() -> bool {
    DMABUF_ACTIF.load(std::sync::atomic::Ordering::Relaxed)
}

static LOGICIEL_ACTIF: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Fixé au démarrage : WebKit dessine-t-il au processeur pour cette session ?
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn set_software_active(actif: bool) {
    LOGICIEL_ACTIF.store(actif, std::sync::atomic::Ordering::Relaxed);
}

pub fn software_active() -> bool {
    LOGICIEL_ACTIF.load(std::sync::atomic::Ordering::Relaxed)
}

#[derive(Debug, Clone)]
pub struct EarlyRenderPrefs {
    pub mode: RenderMode,
    pub dmabuf: String,
    pub gpu_boot_failed: bool,
}

impl Default for EarlyRenderPrefs {
    fn default() -> Self {
        Self {
            mode: RenderMode::Auto,
            dmabuf: "auto".to_string(),
            gpu_boot_failed: false,
        }
    }
}

fn early_prefs_path() -> Option<std::path::PathBuf> {
    Some(
        dirs::data_dir()?
            .join("com.larevuegeek.rustmusic")
            .join("render_prefs"),
    )
}

/// Cache plaintext des préférences de rendu pour lecture avant l'initialisation
/// SQLite et avant tout thread secondaire (Linux `std::env::set_var`).
pub fn load_early_prefs() -> EarlyRenderPrefs {
    let mut prefs = EarlyRenderPrefs::default();
    let Some(path) = early_prefs_path() else {
        return prefs;
    };
    let Ok(raw) = std::fs::read_to_string(path) else {
        return prefs;
    };
    for line in raw.lines() {
        let Some((k, v)) = line.split_once('=') else { continue };
        match k.trim() {
            "render_mode" => prefs.mode = RenderMode::parse_or_auto(Some(v.trim())),
            "dmabuf_mode" => prefs.dmabuf = dmabuf_mode(Some(v.trim())).to_string(),
            "gpu_boot_failed" => prefs.gpu_boot_failed = v.trim() == "1",
            _ => {}
        }
    }
    prefs
}

pub fn save_early_prefs(mode: RenderMode, dmabuf: &str, gpu_boot_failed: bool) {
    let Some(path) = early_prefs_path() else { return };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let content = format!(
        "render_mode={}\ndmabuf_mode={}\ngpu_boot_failed={}\n",
        mode.as_str(),
        dmabuf_mode(Some(dmabuf)),
        if gpu_boot_failed { "1" } else { "0" }
    );
    let _ = std::fs::write(path, content);
}
