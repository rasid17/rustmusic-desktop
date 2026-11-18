//! Shared state between the main Tauri window and the Mobile Remote HTTP server.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tokio::sync::{broadcast, RwLock};
use uuid::Uuid;

const MAX_CONNECTED_CLIENTS: usize = 128;
const MAX_SESSIONS: usize = 64;
const MAX_PIN_FAILURES: u32 = 5;
const PIN_LOCKOUT_SECS: u64 = 30;

/// Génère un code PIN numérique aléatoire à 6 chiffres (100000..=999999).
pub fn generate_random_pin() -> String {
    let raw = u32::from_le_bytes(Uuid::new_v4().as_bytes()[0..4].try_into().unwrap_or([0; 4]));
    format!("{:06}", 100_000 + (raw % 900_000))
}

/// Génère un jeton de session aléatoire (256 bits encodés en hex).
fn generate_session_token() -> String {
    let a = Uuid::new_v4().simple().to_string();
    let b = Uuid::new_v4().simple().to_string();
    format!("{a}{b}")
}

fn constant_time_eq(a: &str, b: &str) -> bool {
    let ab = a.as_bytes();
    let bb = b.as_bytes();
    let max_len = ab.len().max(bb.len());
    let mut diff = ab.len() ^ bb.len();
    for i in 0..max_len {
        let x = ab.get(i).copied().unwrap_or(0);
        let y = bb.get(i).copied().unwrap_or(0);
        diff |= (x ^ y) as usize;
    }
    diff == 0
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthOutcome {
    AllowedNoPin,
    CookieValid,
    PinValid,
    Unauthorized,
    RateLimited,
}

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
    pub sessions: RwLock<VecDeque<String>>,
    pub pin_failures: RwLock<(u32, Option<Instant>)>,
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
            pin_enabled: AtomicBool::new(true),
            pin: RwLock::new(generate_random_pin()),
            sessions: RwLock::new(VecDeque::new()),
            pin_failures: RwLock::new((0, None)),
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
        let cid = client_id.trim();
        if cid.is_empty() || cid.len() > 64 {
            return;
        }
        let mut guard = self.clients.write().await;
        let now = Instant::now();
        guard.retain(|_, last| now.duration_since(*last) < Duration::from_secs(35));
        if !guard.contains_key(cid) && guard.len() >= MAX_CONNECTED_CLIENTS {
            if let Some(oldest_key) = guard
                .iter()
                .min_by_key(|(_, ts)| *ts)
                .map(|(k, _)| k.clone())
            {
                guard.remove(&oldest_key);
            }
        }
        guard.insert(cid.to_string(), now);
    }

    pub async fn client_count(&self) -> usize {
        let mut guard = self.clients.write().await;
        let now = Instant::now();
        guard.retain(|_, last| now.duration_since(*last) < Duration::from_secs(35));
        guard.len()
    }

    pub async fn invalidate_sessions(&self) {
        self.sessions.write().await.clear();
        *self.pin_failures.write().await = (0, None);
    }

    pub async fn issue_session_token(&self) -> String {
        let token = generate_session_token();
        let mut guard = self.sessions.write().await;
        while guard.len() >= MAX_SESSIONS {
            guard.pop_front();
        }
        guard.push_back(token.clone());
        token
    }

    pub async fn authorize_request(
        &self,
        cookie_token: Option<&str>,
        candidate_pin: Option<&str>,
    ) -> AuthOutcome {
        if !self.pin_enabled.load(Ordering::Relaxed) {
            return AuthOutcome::AllowedNoPin;
        }

        if let Some(token) = cookie_token.map(str::trim).filter(|t| !t.is_empty()) {
            let guard = self.sessions.read().await;
            let matched = guard.iter().any(|expected| constant_time_eq(token, expected));
            if matched {
                return AuthOutcome::CookieValid;
            }
        }

        let Some(c) = candidate_pin.map(str::trim).filter(|s| !s.is_empty()) else {
            return AuthOutcome::Unauthorized;
        };

        {
            let mut lock = self.pin_failures.write().await;
            if let Some(until) = lock.1 {
                if Instant::now() < until {
                    return AuthOutcome::RateLimited;
                }
                lock.0 = 0;
                lock.1 = None;
            }
        }

        let expected = self.pin.read().await.clone();
        if constant_time_eq(c, expected.trim()) {
            *self.pin_failures.write().await = (0, None);
            return AuthOutcome::PinValid;
        }

        let mut lock = self.pin_failures.write().await;
        lock.0 = lock.0.saturating_add(1);
        if lock.0 >= MAX_PIN_FAILURES {
            lock.1 = Some(Instant::now() + Duration::from_secs(PIN_LOCKOUT_SECS));
            return AuthOutcome::RateLimited;
        }
        AuthOutcome::Unauthorized
    }

    pub async fn check_pin(&self, candidate: Option<&str>) -> bool {
        matches!(
            self.authorize_request(None, candidate).await,
            AuthOutcome::AllowedNoPin | AuthOutcome::PinValid
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn auth_sessions_lockout_and_client_bounds() {
        let state = MobileRemoteSharedState::new();
        assert!(state.pin_enabled.load(Ordering::Relaxed));
        let pin = state.pin.read().await.clone();
        assert_eq!(pin.len(), 6);

        assert_eq!(
            state.authorize_request(None, Some(&pin)).await,
            AuthOutcome::PinValid
        );
        let cookie = state.issue_session_token().await;
        assert_eq!(
            state.authorize_request(Some(&cookie), None).await,
            AuthOutcome::CookieValid
        );

        state.invalidate_sessions().await;
        assert_eq!(
            state.authorize_request(Some(&cookie), None).await,
            AuthOutcome::Unauthorized
        );

        for _ in 0..4 {
            assert_eq!(
                state.authorize_request(None, Some("000000")).await,
                AuthOutcome::Unauthorized
            );
        }
        assert_eq!(
            state.authorize_request(None, Some("000000")).await,
            AuthOutcome::RateLimited
        );
        assert_eq!(
            state.authorize_request(None, Some(&pin)).await,
            AuthOutcome::RateLimited
        );

        for i in 0..160 {
            state.touch_client(&format!("client-{i}")).await;
        }
        assert_eq!(state.client_count().await, MAX_CONNECTED_CLIENTS);
    }
}
