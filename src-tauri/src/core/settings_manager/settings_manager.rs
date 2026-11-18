use serde::{Deserialize, Serialize};
use std::{fs, io};
use std::{fs::File, path::PathBuf};

#[derive(Serialize, Deserialize)]
pub struct Config {
    pub volume_initial: u8,
    pub device_default: Option<String>,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            volume_initial: 50,
            device_default: None,
        }
    }
}

pub struct SettingsManager;

impl SettingsManager {
    pub fn get_config_file() -> PathBuf {
        // Sur Flatpak, ~/.rust-music peut être sur tmpfs sauf --persist : on privilégie
        // $XDG_CONFIG_HOME (toujours persistant) tout en gardant la compatibilité si
        // l'ancien fichier existe.
        if crate::core::sandbox::flatpak_id().is_some() {
            if let Some(cfg_dir) = dirs::config_dir() {
                let xdg_path = cfg_dir.join("com.larevuegeek.rustmusic").join("config.json");
                let legacy_path = dirs::home_dir()
                    .map(|h| h.join(".rust-music").join("config.json"));
                if !xdg_path.exists() {
                    if let Some(ref legacy) = legacy_path {
                        if legacy.exists() {
                            if let Some(parent) = xdg_path.parent() {
                                let _ = fs::create_dir_all(parent);
                            }
                            let _ = fs::copy(legacy, &xdg_path);
                        }
                    }
                }
                if let Some(parent) = xdg_path.parent() {
                    let _ = fs::create_dir_all(parent);
                }
                return xdg_path;
            }
        }

        let base_dir = dirs::home_dir()
            .or_else(dirs::config_dir)
            .or_else(dirs::data_dir)
            .unwrap_or_else(std::env::temp_dir);

        let config_dir = base_dir.join(".rust-music");
        if !config_dir.is_dir() {
            let _ = fs::create_dir_all(&config_dir);
        }
        config_dir.join("config.json")
    }

    pub fn save_config(config: &Config) -> io::Result<()> {
        let config_path = SettingsManager::get_config_file();

        let file = File::create(config_path)?;
        serde_json::to_writer(file, config)?;

        Ok(())
    }

    /// Config enregistrée, ou les valeurs par défaut si le fichier manque ou ne
    /// se lit pas. Sans ce repli, rien ne créait jamais le fichier : chaque
    /// sauvegarde part d'un chargement, qui échouait. La sortie choisie
    /// retombait alors sur « défaut » à chaque lancement, et le DoP, qui a
    /// besoin de la carte ALSA, cédait sa place au DSD2PCM sans rien dire.
    pub fn load_config() -> io::Result<Config> {
        let config_path = SettingsManager::get_config_file();

        let file = match File::open(&config_path) {
            Ok(file) => file,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Config::default()),
            Err(e) => return Err(e),
        };
        match serde_json::from_reader(file) {
            Ok(config) => Ok(config),
            Err(e) => {
                log::warn!("Config illisible ({}) : {e} — valeurs par défaut", config_path.display());
                Ok(Config::default())
            }
        }
    }
}
