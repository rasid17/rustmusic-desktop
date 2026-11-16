//! Tauri commands exposing the Mobile Remote HTTP server lifecycle and state push.

use std::sync::atomic::Ordering;

use serde::Serialize;
use sqlx::SqlitePool;
use tauri::{AppHandle, State};

use crate::core::dlna_server::net::detect_local_ipv4;
use crate::core::mobile_remote::qr::generate_svg;
use crate::core::mobile_remote::server::MobileRemoteServer;
use crate::core::mobile_remote::state::{MobileRemoteSharedState, RemotePlayerState};
use crate::repository::settings::settings_repository::SettingsRepository;
use crate::state::AppState;

const KEY_ENABLED: &str = "mobile_remote_enabled";
const KEY_PORT: &str = "mobile_remote_port";
const KEY_PIN_ENABLED: &str = "mobile_remote_pin_enabled";
const KEY_PIN: &str = "mobile_remote_pin";

const DEFAULT_PORT: u16 = 8300;
const DEFAULT_PIN: &str = "4829";

#[derive(Debug, Serialize, Clone)]
pub struct MobileRemoteStatus {
    pub running: bool,
    pub port: u16,
    pub pin_enabled: bool,
    pub pin: String,
    pub url: Option<String>,
    pub qr_svg: Option<String>,
    pub connected_clients: usize,
}

#[derive(Debug, Clone)]
struct PersistedConfig {
    enabled: bool,
    port: u16,
    pin_enabled: bool,
    pin: String,
}

async fn load_config(pool: &SqlitePool) -> Result<PersistedConfig, String> {
    let enabled = SettingsRepository::get(pool, KEY_ENABLED)
        .await
        .map_err(|e| format!("read {KEY_ENABLED}: {e}"))?
        .map(|v| v == "true")
        .unwrap_or(false);

    let port = SettingsRepository::get(pool, KEY_PORT)
        .await
        .map_err(|e| format!("read {KEY_PORT}: {e}"))?
        .and_then(|v| v.parse::<u16>().ok())
        .filter(|&p| p > 0)
        .unwrap_or(DEFAULT_PORT);

    let pin_enabled = SettingsRepository::get(pool, KEY_PIN_ENABLED)
        .await
        .map_err(|e| format!("read {KEY_PIN_ENABLED}: {e}"))?
        .map(|v| v == "true")
        .unwrap_or(false);

    let pin = match SettingsRepository::get(pool, KEY_PIN)
        .await
        .map_err(|e| format!("read {KEY_PIN}: {e}"))?
    {
        Some(p) if !p.trim().is_empty() => p,
        _ => {
            SettingsRepository::set(pool, KEY_PIN, DEFAULT_PIN)
                .await
                .map_err(|e| format!("write {KEY_PIN}: {e}"))?;
            DEFAULT_PIN.to_string()
        }
    };

    Ok(PersistedConfig {
        enabled,
        port,
        pin_enabled,
        pin,
    })
}

async fn sync_shared_auth(shared: &MobileRemoteSharedState, cfg: &PersistedConfig) {
    shared.pin_enabled.store(cfg.pin_enabled, Ordering::Relaxed);
    *shared.pin.write().await = cfg.pin.clone();
}

async fn build_status(
    cfg: &PersistedConfig,
    running: bool,
    shared: &MobileRemoteSharedState,
) -> MobileRemoteStatus {
    let (url, qr_svg) = if running {
        let ip = detect_local_ipv4();
        let base_url = format!("http://{}:{}", ip, cfg.port);
        let qr_target = if cfg.pin_enabled {
            format!("{base_url}/?pin={}", urlencoding::encode(&cfg.pin))
        } else {
            format!("{base_url}/")
        };
        (Some(base_url), Some(generate_svg(&qr_target)))
    } else {
        (None, None)
    };

    MobileRemoteStatus {
        running,
        port: cfg.port,
        pin_enabled: cfg.pin_enabled,
        pin: cfg.pin.clone(),
        url,
        qr_svg,
        connected_clients: shared.client_count().await,
    }
}

#[tauri::command]
pub async fn mobile_remote_status(state: State<'_, AppState>) -> Result<MobileRemoteStatus, String> {
    let cfg = load_config(&state.pool).await?;
    sync_shared_auth(&state.mobile_remote_state, &cfg).await;
    let running = state
        .mobile_remote_server
        .lock()
        .await
        .as_ref()
        .is_some_and(|s| s.is_running());
    Ok(build_status(&cfg, running, &state.mobile_remote_state).await)
}

#[tauri::command]
pub async fn mobile_remote_start(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<MobileRemoteStatus, String> {
    let cfg = load_config(&state.pool).await?;
    sync_shared_auth(&state.mobile_remote_state, &cfg).await;

    let mut guard = state.mobile_remote_server.lock().await;
    if let Some(existing) = guard.as_ref() {
        if existing.is_running() {
            return Ok(build_status(&cfg, true, &state.mobile_remote_state).await);
        }
    }

    let mut server = MobileRemoteServer::new(cfg.port);
    server
        .start(
            state.pool.clone(),
            state.mobile_remote_state.clone(),
            app,
        )
        .await?;
    *guard = Some(server);

    SettingsRepository::set(&state.pool, KEY_ENABLED, "true")
        .await
        .map_err(|e| format!("save {KEY_ENABLED}: {e}"))?;

    Ok(build_status(&cfg, true, &state.mobile_remote_state).await)
}

#[tauri::command]
pub async fn mobile_remote_stop(state: State<'_, AppState>) -> Result<MobileRemoteStatus, String> {
    let mut guard = state.mobile_remote_server.lock().await;
    if let Some(mut server) = guard.take() {
        server.stop().await;
    }

    SettingsRepository::set(&state.pool, KEY_ENABLED, "false")
        .await
        .map_err(|e| format!("save {KEY_ENABLED}: {e}"))?;

    let cfg = load_config(&state.pool).await?;
    Ok(build_status(&cfg, false, &state.mobile_remote_state).await)
}

#[tauri::command]
pub async fn mobile_remote_update_settings(
    app: AppHandle,
    state: State<'_, AppState>,
    port: Option<u16>,
    pin_enabled: Option<bool>,
    pin: Option<String>,
) -> Result<MobileRemoteStatus, String> {
    let prev_cfg = load_config(&state.pool).await?;

    if let Some(p) = port {
        if p == 0 {
            return Err("Port must be greater than 0".into());
        }
        SettingsRepository::set(&state.pool, KEY_PORT, &p.to_string())
            .await
            .map_err(|e| format!("save {KEY_PORT}: {e}"))?;
    }

    if let Some(pe) = pin_enabled {
        SettingsRepository::set(&state.pool, KEY_PIN_ENABLED, if pe { "true" } else { "false" })
            .await
            .map_err(|e| format!("save {KEY_PIN_ENABLED}: {e}"))?;
    }

    if let Some(ref new_pin) = pin {
        let trimmed = new_pin.trim();
        if trimmed.is_empty() {
            return Err("PIN cannot be empty".into());
        }
        SettingsRepository::set(&state.pool, KEY_PIN, trimmed)
            .await
            .map_err(|e| format!("save {KEY_PIN}: {e}"))?;
    }

    let cfg = load_config(&state.pool).await?;
    sync_shared_auth(&state.mobile_remote_state, &cfg).await;

    let mut guard = state.mobile_remote_server.lock().await;
    let was_running = guard.as_ref().is_some_and(|s| s.is_running());

    if was_running && cfg.port != prev_cfg.port {
        if let Some(mut old_server) = guard.take() {
            old_server.stop().await;
        }
        let mut new_server = MobileRemoteServer::new(cfg.port);
        new_server
            .start(
                state.pool.clone(),
                state.mobile_remote_state.clone(),
                app,
            )
            .await?;
        *guard = Some(new_server);
    }

    Ok(build_status(&cfg, was_running, &state.mobile_remote_state).await)
}

#[tauri::command]
pub async fn mobile_remote_push_state(
    state: State<'_, AppState>,
    payload: RemotePlayerState,
) -> Result<(), String> {
    state.mobile_remote_state.update_player(payload).await;
    Ok(())
}

pub async fn auto_start_if_enabled(state: &AppState, app: AppHandle) {
    let cfg = match load_config(&state.pool).await {
        Ok(c) => c,
        Err(e) => {
            log::warn!("Mobile Remote auto-start: cannot load settings ({e})");
            return;
        }
    };
    sync_shared_auth(&state.mobile_remote_state, &cfg).await;

    if !cfg.enabled {
        return;
    }

    let mut server = MobileRemoteServer::new(cfg.port);
    if let Err(e) = server
        .start(
            state.pool.clone(),
            state.mobile_remote_state.clone(),
            app,
        )
        .await
    {
        log::error!("Mobile Remote auto-start failed: {e}");
        return;
    }
    *state.mobile_remote_server.lock().await = Some(server);
}
