//! Client HTTP untuk API TIDAL (OAuth 2.0 Device Flow, token refresh, sesi pengguna).
//!
//! Mendukung login via browser (Device Authorization Flow) maupun token langsung
//! (`access_token` atau `refresh_token`), serta pembaruan token otomatis ketika kedaluwarsa.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use reqwest::header::{AUTHORIZATION, CONTENT_TYPE};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::core::podcast::fetch::client;
use crate::core::subsonic::client::{MusicFolder, ServerInfo};

pub const API_BASE: &str = "https://api.tidal.com/v1";
const AUTH_BASE: &str = "https://auth.tidal.com/v1/oauth2";
const TIMEOUT: Duration = Duration::from_secs(20);

/// Kredensial OAuth publik standar untuk Device Authorization Flow TIDAL.
const DEFAULT_CLIENT_ID: &str = "zU4XHVVkc2tDPo4t";
const DEFAULT_CLIENT_SECRET: &str = "VJKhDFqJPqvsPVNBV6ukXTJmwlvbttP7wlMlrc72se4=";

/// Hasil awal Device Authorization Flow untuk ditampilkan kepada pengguna.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TidalDeviceAuth {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    pub verification_uri_complete: String,
    pub expires_in: u64,
    pub interval: u64,
}

/// Hasil polling status otorisasi perangkat TIDAL.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TidalDevicePoll {
    pub status: String,
    pub access_token: Option<String>,
    pub refresh_token: Option<String>,
    pub username: Option<String>,
    pub user_id: Option<String>,
    pub country_code: Option<String>,
}

#[derive(Debug, Clone)]
struct CachedToken {
    access_token: String,
    refresh_token: String,
}

fn token_cache() -> &'static Mutex<HashMap<String, CachedToken>> {
    static CACHE: OnceLock<Mutex<HashMap<String, CachedToken>>> = OnceLock::new();
    CACHE.get_or_init(Default::default)
}

#[derive(Debug, Clone)]
pub struct Server {
    pub base: String,
    pub username: String,
    pub access_token: String,
    pub refresh_token: String,
    pub client_id: String,
    pub client_secret: String,
    pub user_id: String,
    pub country_code: String,
}

impl Server {
    /// Rekonstruksi dari baris tersimpan di `media_servers`.
    pub fn from_parts(
        url: &str,
        username: &str,
        access_token: &str,
        refresh_token: &str,
        api_key: &str,
        stored_user_id: &str,
    ) -> Self {
        let base = if url.trim().is_empty() {
            API_BASE.to_string()
        } else {
            url.trim_end_matches('/').to_string()
        };
        let (client_id, client_secret) = parse_client_creds(Some(api_key));
        let (user_id, country_code) = match stored_user_id.split_once(':') {
            Some((uid, cc)) => (
                uid.trim().to_string(),
                if cc.trim().is_empty() { "US".to_string() } else { cc.trim().to_string() },
            ),
            None => (stored_user_id.trim().to_string(), "US".to_string()),
        };
        Self {
            base,
            username: username.to_string(),
            access_token: access_token.to_string(),
            refresh_token: refresh_token.to_string(),
            client_id,
            client_secret,
            user_id,
            country_code,
        }
    }

    pub fn api_key_string(&self) -> String {
        if self.client_secret.is_empty() {
            self.client_id.clone()
        } else {
            format!("{}:{}", self.client_id, self.client_secret)
        }
    }

    pub fn stored_user_id(&self) -> String {
        format!("{}:{}", self.user_id, self.country_code)
    }

    fn cache_key(&self) -> String {
        format!("{}:{}", self.user_id, self.country_code)
    }

    /// Ambil `access_token` terbaru dari cache runtime bila sudah diperbarui oleh refresh otomatis.
    pub fn effective_access_token(&self) -> String {
        let key = self.cache_key();
        if !self.user_id.is_empty() {
            if let Some(entry) = token_cache().lock().unwrap_or_else(|e| e.into_inner()).get(&key) {
                return entry.access_token.clone();
            }
        }
        self.access_token.clone()
    }

    pub fn effective_refresh_token(&self) -> String {
        let key = self.cache_key();
        if !self.user_id.is_empty() {
            if let Some(entry) = token_cache().lock().unwrap_or_else(|e| e.into_inner()).get(&key) {
                if !entry.refresh_token.is_empty() {
                    return entry.refresh_token.clone();
                }
            }
        }
        self.refresh_token.clone()
    }

    fn remember_tokens(&self, access: &str, refresh: &str) {
        if self.user_id.is_empty() {
            return;
        }
        token_cache().lock().unwrap_or_else(|e| e.into_inner()).insert(
            self.cache_key(),
            CachedToken {
                access_token: access.to_string(),
                refresh_token: refresh.to_string(),
            },
        );
    }

    /// Hubungkan ke akun TIDAL dari:
    /// - `tokens:<access_token>|<refresh_token>` (hasil OAuth Device Flow), atau
    /// - `access_token` langsung, atau
    /// - `refresh_token` langsung.
    pub async fn connect(
        url: &str,
        username: &str,
        secret: &str,
        api_key: Option<&str>,
    ) -> Result<Self, String> {
        let base = if url.trim().is_empty() {
            API_BASE.to_string()
        } else {
            url.trim_end_matches('/').to_string()
        };
        let (client_id, client_secret) = parse_client_creds(api_key);
        let secret = secret.trim();
        if secret.is_empty() {
            return Err("Sign in with TIDAL or paste an access/refresh token".into());
        }

        let (access_token, refresh_token) = if let Some(pair) = secret.strip_prefix("tokens:") {
            let (acc, ref_tok) = pair.split_once('|').unwrap_or((pair, ""));
            (acc.trim().to_string(), ref_tok.trim().to_string())
        } else {
            // Coba sebagai access_token lebih dulu; bila ditolak (401/403), coba refresh_token.
            match fetch_session(&base, secret).await {
                Ok(_) => (secret.to_string(), String::new()),
                Err(_) => {
                    let refreshed =
                        refresh_with_token(&client_id, &client_secret, secret).await?;
                    (refreshed.0, refreshed.1)
                }
            }
        };

        let session = match fetch_session(&base, &access_token).await {
            Ok(s) => (s, access_token, refresh_token),
            Err(e) if !refresh_token.is_empty() => {
                let (new_acc, new_ref) =
                    refresh_with_token(&client_id, &client_secret, &refresh_token).await?;
                let s = fetch_session(&base, &new_acc).await.map_err(|_| e)?;
                (s, new_acc, new_ref)
            }
            Err(e) => return Err(e),
        };

        let (info, final_access, final_refresh) = session;
        let final_user = if !info.username.is_empty() {
            info.username
        } else if !username.trim().is_empty() {
            username.trim().to_string()
        } else {
            format!("tidal-{}", info.user_id)
        };

        let srv = Self {
            base,
            username: final_user,
            access_token: final_access.clone(),
            refresh_token: final_refresh.clone(),
            client_id,
            client_secret,
            user_id: info.user_id,
            country_code: info.country_code,
        };
        srv.remember_tokens(&final_access, &final_refresh);
        Ok(srv)
    }

    /// Mulai OAuth 2.0 Device Authorization Flow.
    pub async fn device_code_start(custom_client: Option<&str>) -> Result<TidalDeviceAuth, String> {
        let (client_id, _) = parse_client_creds(custom_client);
        let body = form_body(&[("client_id", &client_id), ("scope", "r_usr w_usr w_sub")]);
        let resp = client()
            .post(format!("{AUTH_BASE}/device_authorization"))
            .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
            .body(body)
            .timeout(TIMEOUT)
            .send()
            .await
            .map_err(|e| format!("Failed to reach TIDAL auth: {e}"))?;

        let status = resp.status();
        let val: Value = resp
            .json()
            .await
            .map_err(|e| format!("Invalid response from TIDAL auth: {e}"))?;
        if !status.is_success() {
            return Err(extract_error(&val, status.as_u16()));
        }

        let device_code = val
            .get("deviceCode")
            .and_then(Value::as_str)
            .ok_or("Missing deviceCode in TIDAL response")?
            .to_string();
        let user_code = val
            .get("userCode")
            .and_then(Value::as_str)
            .ok_or("Missing userCode in TIDAL response")?
            .to_string();
        let raw_uri = val
            .get("verificationUri")
            .and_then(Value::as_str)
            .unwrap_or("https://link.tidal.com");
        let raw_complete = val
            .get("verificationUriComplete")
            .and_then(Value::as_str)
            .unwrap_or(raw_uri);

        Ok(TidalDeviceAuth {
            device_code,
            user_code,
            verification_uri: normalize_https(raw_uri),
            verification_uri_complete: normalize_https(raw_complete),
            expires_in: val.get("expiresIn").and_then(Value::as_u64).unwrap_or(300),
            interval: val.get("interval").and_then(Value::as_u64).unwrap_or(2).max(1),
        })
    }

    /// Polling token hasil otorisasi browser.
    pub async fn device_code_poll(
        device_code: &str,
        custom_client: Option<&str>,
    ) -> Result<TidalDevicePoll, String> {
        let (client_id, client_secret) = parse_client_creds(custom_client);
        let mut pairs = vec![
            ("client_id", client_id.as_str()),
            ("device_code", device_code.trim()),
            ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
            ("scope", "r_usr w_usr w_sub"),
        ];
        if !client_secret.is_empty() {
            pairs.push(("client_secret", client_secret.as_str()));
        }
        let resp = client()
            .post(format!("{AUTH_BASE}/token"))
            .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
            .body(form_body(&pairs))
            .timeout(TIMEOUT)
            .send()
            .await
            .map_err(|e| format!("Failed to reach TIDAL token endpoint: {e}"))?;

        let status = resp.status();
        let val: Value = resp
            .json()
            .await
            .map_err(|e| format!("Invalid JSON from TIDAL token endpoint: {e}"))?;

        if !status.is_success() {
            let err_code = val.get("error").and_then(Value::as_str).unwrap_or("");
            if err_code == "authorization_pending" || err_code == "slow_down" {
                return Ok(TidalDevicePoll {
                    status: "pending".into(),
                    access_token: None,
                    refresh_token: None,
                    username: None,
                    user_id: None,
                    country_code: None,
                });
            }
            if err_code == "expired_token" {
                return Err("TIDAL login session expired. Please start sign-in again.".into());
            }
            return Err(extract_error(&val, status.as_u16()));
        }

        let access_token = val
            .get("access_token")
            .and_then(Value::as_str)
            .ok_or("Missing access_token in TIDAL response")?
            .to_string();
        let refresh_token = val
            .get("refresh_token")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();

        let session = fetch_session(API_BASE, &access_token).await.ok();
        let user_obj = val.get("user");
        let uid = session
            .as_ref()
            .map(|s| s.user_id.clone())
            .or_else(|| user_obj.and_then(|u| u.get("userId")).and_then(json_id));
        let cc = session
            .as_ref()
            .map(|s| s.country_code.clone())
            .or_else(|| {
                user_obj
                    .and_then(|u| u.get("countryCode"))
                    .and_then(Value::as_str)
                    .map(str::to_string)
            });
        let uname = session
            .as_ref()
            .map(|s| s.username.clone())
            .filter(|u| !u.is_empty())
            .or_else(|| {
                user_obj
                    .and_then(|u| {
                        u.get("username")
                            .or_else(|| u.get("email"))
                            .and_then(Value::as_str)
                    })
                    .map(str::to_string)
            });

        Ok(TidalDevicePoll {
            status: "authorized".into(),
            access_token: Some(access_token),
            refresh_token: (!refresh_token.is_empty()).then_some(refresh_token),
            username: uname,
            user_id: uid,
            country_code: cc,
        })
    }

    pub fn build_url(&self, path: &str, params: &[(&str, &str)]) -> Result<reqwest::Url, String> {
        let base = format!("{}/{}", self.base.trim_end_matches('/'), path.trim_start_matches('/'));
        let mut owned: Vec<(&str, &str)> = params.to_vec();
        if !owned.iter().any(|(k, _)| *k == "countryCode") && !self.country_code.is_empty() {
            owned.push(("countryCode", &self.country_code));
        }
        reqwest::Url::parse_with_params(&base, &owned)
            .map_err(|e| format!("Invalid TIDAL URL: {e}"))
    }

    /// GET JSON terautentikasi, otomatis mencoba refresh token 1x bila 401.
    pub async fn get_json(&self, path: &str, params: &[(&str, &str)]) -> Result<Value, String> {
        let url = self.build_url(path, params)?;
        let mut token = self.effective_access_token();
        for attempt in 0..2 {
            let resp = client()
                .get(url.clone())
                .header(AUTHORIZATION, format!("Bearer {token}"))
                .timeout(TIMEOUT)
                .send()
                .await
                .map_err(|e| format!("TIDAL request failed: {e}"))?;
            let status = resp.status();
            if status == reqwest::StatusCode::UNAUTHORIZED && attempt == 0 {
                let ref_tok = self.effective_refresh_token();
                if !ref_tok.is_empty() {
                    if let Ok((new_acc, new_ref)) =
                        refresh_with_token(&self.client_id, &self.client_secret, &ref_tok).await
                    {
                        self.remember_tokens(&new_acc, &new_ref);
                        token = new_acc;
                        continue;
                    }
                }
            }
            let text = resp.text().await.unwrap_or_default();
            let val: Value = serde_json::from_str(&text)
                .unwrap_or_else(|_| Value::String(text.clone()));
            if !status.is_success() {
                return Err(extract_error(&val, status.as_u16()));
            }
            return Ok(val);
        }
        Err("TIDAL authentication expired; please reconnect your account".into())
    }

    pub async fn post_form(
        &self,
        path: &str,
        params: &[(&str, &str)],
        form: &[(&str, &str)],
        etag: Option<&str>,
    ) -> Result<(Value, Option<String>), String> {
        let url = self.build_url(path, params)?;
        let mut token = self.effective_access_token();
        let body = form_body(form);
        for attempt in 0..2 {
            let mut req = client()
                .post(url.clone())
                .header(AUTHORIZATION, format!("Bearer {token}"))
                .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(body.clone())
                .timeout(TIMEOUT);
            if let Some(tag) = etag {
                req = req.header("If-None-Match", tag);
            }
            let resp = req
                .send()
                .await
                .map_err(|e| format!("TIDAL POST failed: {e}"))?;
            let status = resp.status();
            if status == reqwest::StatusCode::UNAUTHORIZED && attempt == 0 {
                let ref_tok = self.effective_refresh_token();
                if !ref_tok.is_empty() {
                    if let Ok((new_acc, new_ref)) =
                        refresh_with_token(&self.client_id, &self.client_secret, &ref_tok).await
                    {
                        self.remember_tokens(&new_acc, &new_ref);
                        token = new_acc;
                        continue;
                    }
                }
            }
            let next_etag = resp
                .headers()
                .get("ETag")
                .and_then(|v| v.to_str().ok())
                .map(str::to_string);
            let text = resp.text().await.unwrap_or_default();
            let val: Value = if text.trim().is_empty() {
                Value::Null
            } else {
                serde_json::from_str(&text).unwrap_or(Value::String(text))
            };
            if !status.is_success() {
                return Err(extract_error(&val, status.as_u16()));
            }
            return Ok((val, next_etag));
        }
        Err("TIDAL authentication expired; please reconnect your account".into())
    }

    pub async fn delete_req(
        &self,
        path: &str,
        params: &[(&str, &str)],
        etag: Option<&str>,
    ) -> Result<(), String> {
        let url = self.build_url(path, params)?;
        let mut token = self.effective_access_token();
        for attempt in 0..2 {
            let mut req = client()
                .delete(url.clone())
                .header(AUTHORIZATION, format!("Bearer {token}"))
                .timeout(TIMEOUT);
            if let Some(tag) = etag {
                req = req.header("If-None-Match", tag);
            }
            let resp = req
                .send()
                .await
                .map_err(|e| format!("TIDAL DELETE failed: {e}"))?;
            let status = resp.status();
            if status == reqwest::StatusCode::UNAUTHORIZED && attempt == 0 {
                let ref_tok = self.effective_refresh_token();
                if !ref_tok.is_empty() {
                    if let Ok((new_acc, new_ref)) =
                        refresh_with_token(&self.client_id, &self.client_secret, &ref_tok).await
                    {
                        self.remember_tokens(&new_acc, &new_ref);
                        token = new_acc;
                        continue;
                    }
                }
            }
            if !status.is_success() {
                let text = resp.text().await.unwrap_or_default();
                let val: Value = serde_json::from_str(&text).unwrap_or(Value::String(text));
                return Err(extract_error(&val, status.as_u16()));
            }
            return Ok(());
        }
        Err("TIDAL authentication expired; please reconnect your account".into())
    }

    pub async fn ping(&self) -> Result<ServerInfo, String> {
        let val = self.get_json("sessions", &[]).await?;
        let country = val
            .get("countryCode")
            .and_then(Value::as_str)
            .unwrap_or(&self.country_code);
        let sub = self
            .get_json(&format!("users/{}/subscription", self.user_id), &[])
            .await
            .ok();
        let tier = sub
            .as_ref()
            .and_then(|s| s.pointer("/subscription/type").and_then(Value::as_str))
            .unwrap_or("HiFi Plus / MAX");
        Ok(ServerInfo {
            kind: Some("tidal".into()),
            name: Some(if self.username.trim().is_empty() {
                "TIDAL Hi-Res".into()
            } else {
                format!("TIDAL ({})", self.username.trim())
            }),
            device_id: None,
            version: Some(format!("{country} · {tier}")),
            open_subsonic: false,
            extensions: vec!["bit-perfect-flac".into(), "hi-res-24-192".into(), "dash-flac".into()],
        })
    }

    pub async fn music_folders(&self) -> Result<Vec<MusicFolder>, String> {
        Ok(vec![
            MusicFolder {
                id: "favorites".into(),
                name: "TIDAL — Favorites (Albums & Tracks)".into(),
            },
            MusicFolder {
                id: "playlists".into(),
                name: "TIDAL — Playlists Tracks".into(),
            },
        ])
    }
}

/// Bangun URL gambar sampul TIDAL dari UUID (`xxxxxxxx-xxxx-...`).
pub fn cover_art_url(cover_id: &str, max_size: Option<u32>) -> Option<String> {
    let trimmed = cover_id.trim();
    if trimmed.is_empty() {
        return None;
    }
    if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        return Some(trimmed.to_string());
    }
    let dim = match max_size {
        Some(s) if s <= 320 => "320x320",
        Some(s) if s > 800 => "1280x1280",
        _ => "640x640",
    };
    let path = trimmed.replace('-', "/");
    Some(format!("https://resources.tidal.com/images/{path}/{dim}.jpg"))
}

struct SessionInfo {
    user_id: String,
    country_code: String,
    username: String,
}

async fn fetch_session(base: &str, access_token: &str) -> Result<SessionInfo, String> {
    let url = format!("{}/sessions", base.trim_end_matches('/'));
    let resp = client()
        .get(&url)
        .header(AUTHORIZATION, format!("Bearer {}", access_token.trim()))
        .timeout(TIMEOUT)
        .send()
        .await
        .map_err(|e| format!("Failed to reach TIDAL sessions: {e}"))?;
    let status = resp.status();
    let val: Value = resp
        .json()
        .await
        .map_err(|e| format!("Invalid response from TIDAL sessions: {e}"))?;
    if !status.is_success() {
        return Err(extract_error(&val, status.as_u16()));
    }
    let user_id = val
        .get("userId")
        .and_then(json_id)
        .ok_or("Missing userId in TIDAL session")?;
    let country_code = val
        .get("countryCode")
        .and_then(Value::as_str)
        .unwrap_or("US")
        .to_string();
    let username = val
        .get("username")
        .or_else(|| val.get("email"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    Ok(SessionInfo {
        user_id,
        country_code,
        username,
    })
}

async fn refresh_with_token(
    client_id: &str,
    client_secret: &str,
    refresh_token: &str,
) -> Result<(String, String), String> {
    let mut pairs = vec![
        ("client_id", client_id),
        ("refresh_token", refresh_token.trim()),
        ("grant_type", "refresh_token"),
        ("scope", "r_usr w_usr w_sub"),
    ];
    if !client_secret.is_empty() {
        pairs.push(("client_secret", client_secret));
    }
    let resp = client()
        .post(format!("{AUTH_BASE}/token"))
        .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(form_body(&pairs))
        .timeout(TIMEOUT)
        .send()
        .await
        .map_err(|e| format!("Failed to refresh TIDAL token: {e}"))?;
    let status = resp.status();
    let val: Value = resp
        .json()
        .await
        .map_err(|e| format!("Invalid TIDAL refresh response: {e}"))?;
    if !status.is_success() {
        return Err(extract_error(&val, status.as_u16()));
    }
    let access = val
        .get("access_token")
        .and_then(Value::as_str)
        .ok_or("Missing access_token after refresh")?
        .to_string();
    let next_refresh = val
        .get("refresh_token")
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| refresh_token.to_string());
    Ok((access, next_refresh))
}

pub(crate) fn parse_client_creds(api_key: Option<&str>) -> (String, String) {
    let trimmed = api_key.unwrap_or("").trim();
    if trimmed.is_empty() {
        return (
            DEFAULT_CLIENT_ID.to_string(),
            DEFAULT_CLIENT_SECRET.to_string(),
        );
    }
    if let Some((id, secret)) = trimmed.split_once(':') {
        let id = id.trim();
        let secret = secret.trim();
        if !id.is_empty() {
            return (id.to_string(), secret.to_string());
        }
    }
    if trimmed == DEFAULT_CLIENT_ID {
        return (
            DEFAULT_CLIENT_ID.to_string(),
            DEFAULT_CLIENT_SECRET.to_string(),
        );
    }
    (trimmed.to_string(), String::new())
}

fn normalize_https(uri: &str) -> String {
    let trimmed = uri.trim();
    if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        trimmed.to_string()
    } else {
        format!("https://{}", trimmed.trim_start_matches('/'))
    }
}

pub(crate) fn json_id(v: &Value) -> Option<String> {
    v.as_str()
        .map(|s| s.to_string())
        .or_else(|| v.as_u64().map(|n| n.to_string()))
        .or_else(|| v.as_i64().map(|n| n.to_string()))
}

pub(crate) fn form_body(pairs: &[(&str, &str)]) -> String {
    pairs
        .iter()
        .map(|(k, v)| format!("{}={}", urlencoding::encode(k), urlencoding::encode(v)))
        .collect::<Vec<_>>()
        .join("&")
}

fn extract_error(v: &Value, status: u16) -> String {
    let msg = v
        .get("userMessage")
        .or_else(|| v.get("error_description"))
        .or_else(|| v.get("message"))
        .or_else(|| v.get("error"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty());
    match msg {
        Some(m) => format!("TIDAL API ({status}): {m}"),
        None => format!("TIDAL HTTP error {status}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_tidal_cover_url_from_uuid() {
        assert_eq!(
            cover_art_url("a1b2c3d4-e5f6-4789-abcd-ef0123456789", None),
            Some("https://resources.tidal.com/images/a1b2c3d4/e5f6/4789/abcd/ef0123456789/640x640.jpg".into())
        );
        assert_eq!(
            cover_art_url("a1b2c3d4-e5f6-4789-abcd-ef0123456789", Some(1280)),
            Some("https://resources.tidal.com/images/a1b2c3d4/e5f6/4789/abcd/ef0123456789/1280x1280.jpg".into())
        );
    }

    #[test]
    fn parses_stored_user_id_and_client_creds() {
        let srv = Server::from_parts("", "user@test.com", "acc", "ref", "", "998877:ID");
        assert_eq!(srv.user_id, "998877");
        assert_eq!(srv.country_code, "ID");
        assert_eq!(srv.client_id, DEFAULT_CLIENT_ID);
        assert_eq!(srv.stored_user_id(), "998877:ID");
    }
}
