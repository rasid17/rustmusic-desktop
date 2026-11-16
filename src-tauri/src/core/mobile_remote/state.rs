//! Shared state between the main Tauri window and the Mobile Remote HTTP server.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tokio::sync::{broadcast, RwLock};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RemoteQueueItem {
    pub queue_id: String,
    pub index: usize,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub path: String,
    pub duration: f64,
    pub thumbnail_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemotePlayerState {
    pub status: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub path: Option<String>,
    pub track_id: Option<String>,
    pub duration: f64,
    pub position: f64,
    pub volume: u8,
    pub muted: bool,
    pub dop_active: bool,
    pub is_shuffled: bool,
    pub repeat_mode: String,
    pub liked: bool,
    pub thumbnail_path: Option<String>,
    pub audio_format: Option<String>,
    pub bits_per_sample: Option<i32>,
    pub sample_rate: Option<i32>,
    pub bitrate: Option<i32>,
    pub pipeline_label: Option<String>,
    pub current_index: i64,
    pub queue: Vec<RemoteQueueItem>,
    pub updated_at_ms: u64,
}

impl Default for RemotePlayerState {
    fn default() -> Self {
        Self {
            status: "stopped".to_string(),
            title: String::new(),
            artist: String::new(),
            album: String::new(),
            path: None,
            track_id: None,
            duration: 0.0,
            position: 0.0,
            volume: 100,
            muted: false,
            dop_active: false,
            is_shuffled: false,
            repeat_mode: "off".to_string(),
            liked: false,
            thumbnail_path: None,
            audio_format: None,
            bits_per_sample: None,
            sample_rate: None,
            bitrate: None,
            pipeline_label: None,
            current_index: -1,
            queue: Vec::new(),
            updated_at_ms: 0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteCmdPayload {
    pub cmd: String,
    #[serde(default)]
    pub position: Option<f64>,
    #[serde(default)]
    pub volume: Option<u8>,
    #[serde(default)]
    pub index: Option<usize>,
    #[serde(default)]
    pub from: Option<usize>,
    #[serde(default)]
    pub to: Option<usize>,
    #[serde(default)]
    pub queue_id: Option<String>,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub paths: Option<Vec<String>>,
    #[serde(default)]
    pub track_id: Option<String>,
    #[serde(default)]
    pub album_id: Option<String>,
    #[serde(default)]
    pub artist_id: Option<String>,
    #[serde(default)]
    pub playlist_id: Option<String>,
    #[serde(default)]
    pub shuffle: Option<bool>,
}

pub struct MobileRemoteSharedState {
    pub player: RwLock<RemotePlayerState>,
    pub state_tx: broadcast::Sender<u64>,
    pub clients: RwLock<HashMap<String, Instant>>,
    pub pin_enabled: AtomicBool,
    pub pin: RwLock<String>,
}

impl Default for MobileRemoteSharedState {
    fn default() -> Self {
        Self::new()
    }
}

impl MobileRemoteSharedState {
    pub fn new() -> Self {
        let (state_tx, _) = broadcast::channel(64);
        Self {
            player: RwLock::new(RemotePlayerState::default()),
            state_tx,
            clients: RwLock::new(HashMap::new()),
            pin_enabled: AtomicBool::new(false),
            pin: RwLock::new("1234".to_string()),
        }
    }

    pub async fn update_player(&self, mut next: RemotePlayerState) {
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(1);
        next.updated_at_ms = now_ms;
        *self.player.write().await = next;
        let _ = self.state_tx.send(now_ms);
    }

    pub async fn snapshot(&self) -> RemotePlayerState {
        self.player.read().await.clone()
    }

    pub async fn touch_client(&self, client_id: &str) {
        if client_id.is_empty() {
            return;
        }
        let mut guard = self.clients.write().await;
        let now = Instant::now();
        guard.retain(|_, last| now.duration_since(*last) < Duration::from_secs(35));
        guard.insert(client_id.to_string(), now);
    }

    pub async fn client_count(&self) -> usize {
        let mut guard = self.clients.write().await;
        let now = Instant::now();
        guard.retain(|_, last| now.duration_since(*last) < Duration::from_secs(35));
        guard.len()
    }

    pub async fn check_pin(&self, candidate: Option<&str>) -> bool {
        if !self.pin_enabled.load(Ordering::Relaxed) {
            return true;
        }
        let Some(c) = candidate else {
            return false;
        };
        let expected = self.pin.read().await;
        c.trim() == expected.trim()
    }
}
