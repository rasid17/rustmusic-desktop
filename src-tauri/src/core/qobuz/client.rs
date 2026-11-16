//! Client Qobuz : extraction automatique de `app_id` et `app_secret` depuis le lecteur web
//! (avec valeurs de secours intégrées), connexion par e-mail/mot de passe ou par jeton
//! (`user_auth_token`), et signature MD5 des requêtes de flux Hi-Res FLAC (jusqu'à 24 bits / 192 kHz).

use std::sync::OnceLock;
use std::time::Duration;

use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use regex::Regex;
use serde_json::Value;
use tokio::sync::Mutex;

use crate::core::podcast::fetch::client;
use crate::core::subsonic::client::{MusicFolder, ServerInfo};

pub const API_BASE: &str = "https://www.qobuz.com/api.json/0.2";
pub const STORED_URL: &str = "qobuz://www.qobuz.com/api.json/0.2";
const WEB_LOGIN_URL: &str = "https://play.qobuz.com/login";
const WEB_ROOT_URL: &str = "https://play.qobuz.com";

pub const DEFAULT_APP_ID: &str = "950096963";
pub const DEFAULT_APP_SECRET: &str = "10b251c2868bf6d96c3f6f7fb0009093";

const FALLBACK_SECRETS: &[(&str, &str)] = &[
    ("950096963", "10b251c2868bf6d96c3f6f7fb0009093"),
    ("950096963", "abb21364945c0583309667d13ca3d93a"),
    ("950096963", "e1ee9e6a0af1e9e6343c715587710cc2"),
    ("798273057", "abb21364945c0583309667d13ca3d93a"),
    ("798273057", "806331c3b0b641da923b890aed01d04a"),
];

/// Cache en mémoire des identifiants d'application Qobuz extraits du bundle web.
static DISCOVERED_CREDS: OnceLock<Mutex<Option<(String, Vec<String>)>>> = OnceLock::new();

fn creds_cache() -> &'static Mutex<Option<(String, Vec<String>)>> {
    DISCOVERED_CREDS.get_or_init(|| Mutex::new(None))
}

/// Compte Qobuz connecté : identifiants d'application, jeton utilisateur et identifiant de compte.
#[derive(Debug, Clone, PartialEq)]
pub struct Server {
    pub base: String,
    pub app_id: String,
    pub app_secret: String,
    pub token: String,
    pub user_id: String,
}

/// Extrait `app_id` et la liste des `app_secret` candidats depuis le code d'un `bundle.js` Qobuz.
pub fn extract_bundle_credentials(bundle: &str) -> Option<(String, Vec<String>)> {
    let app_id = Regex::new(r#"production\s*:\s*\{\s*api\s*:\s*\{\s*appId\s*:\s*"(?P<id>\d{9})""#)
        .ok()
        .and_then(|re| re.captures(bundle))
        .and_then(|c| c.name("id").map(|m| m.as_str().to_string()))
        .or_else(|| {
            Regex::new(r#"app_id\s*[:=]\s*"(?P<id>\d{9})""#)
                .ok()
                .and_then(|re| re.captures(bundle))
                .and_then(|c| c.name("id").map(|m| m.as_str().to_string()))
        })?;

    let mut secrets = Vec::new();

    if let Ok(direct_re) = Regex::new(r#"appSecret\s*:\s*"(?P<secret>[a-fA-F0-9]{32})"|app_secret\s*:\s*"(?P<secret2>[a-fA-F0-9]{32})""#) {
        for cap in direct_re.captures_iter(bundle) {
            if let Some(m) = cap.name("secret").or_else(|| cap.name("secret2")) {
                let s = m.as_str().to_string();
                if !secrets.contains(&s) {
                    secrets.push(s);
                }
            }
        }
    }

    if let Ok(seed_re) = Regex::new(r#"[a-z]\.initialSeed\("(?P<seed>[\w=]+)",window\.utimezone\.(?P<tz>[a-z]+)\)"#) {
        for cap in seed_re.captures_iter(bundle) {
            let (Some(seed), Some(tz)) = (cap.name("seed").map(|m| m.as_str()), cap.name("tz").map(|m| m.as_str())) else {
                continue;
            };
            let tz_pattern = format!(
                r#"name\s*:\s*"\w+/(?i:{})"\s*,\s*info\s*:\s*"(?P<info>[\w=]+)"\s*,\s*extras\s*:\s*"(?P<extras>[\w=]+)""#,
                regex::escape(tz)
            );
            if let Ok(info_re) = Regex::new(&tz_pattern) {
                if let Some(info_cap) = info_re.captures(bundle) {
                    let info = info_cap.name("info").map(|m| m.as_str()).unwrap_or_default();
                    let extras = info_cap.name("extras").map(|m| m.as_str()).unwrap_or_default();
                    let combined = format!("{seed}{info}{extras}");
                    if combined.len() > 44 {
                        let trimmed = &combined[..combined.len() - 44];
                        if let Ok(decoded) = BASE64.decode(trimmed.as_bytes()) {
                            if let Ok(secret) = String::from_utf8(decoded) {
                                let secret = secret.trim().to_string();
                                if secret.len() == 32 && !secrets.contains(&secret) {
                                    secrets.push(secret);
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    if secrets.is_empty() {
        return None;
    }
    Some((app_id, secrets))
}

/// Récupère les identifiants d'application Qobuz depuis `play.qobuz.com` (ou le cache / valeurs de secours).
pub async fn discover_app_credentials() -> (String, Vec<String>) {
    {
        let guard = creds_cache().lock().await;
        if let Some(cached) = guard.as_ref() {
            return cached.clone();
        }
    }

    let fetched = async {
        let login_html = client()
            .get(WEB_LOGIN_URL)
            .timeout(Duration::from_secs(12))
            .send()
            .await
            .ok()?
            .text()
            .await
            .ok()?;
        let script_re = Regex::new(r#"<script[^>]+src="(/resources/[^"]+/bundle\.js)""#).ok()?;
        let bundle_path = script_re.captures(&login_html)?.get(1)?.as_str();
        let bundle_js = client()
            .get(format!("{WEB_ROOT_URL}{bundle_path}"))
            .timeout(Duration::from_secs(20))
            .send()
            .await
            .ok()?
            .text()
            .await
            .ok()?;
        extract_bundle_credentials(&bundle_js)
    }
    .await;

    let result = match fetched {
        Some((app_id, mut secrets)) => {
            for (_, s) in FALLBACK_SECRETS {
                if !secrets.iter().any(|x| x == s) {
                    secrets.push((*s).to_string());
                }
            }
            (app_id, secrets)
        }
        None => {
            let secrets = FALLBACK_SECRETS.iter().map(|(_, s)| (*s).to_string()).collect();
            (DEFAULT_APP_ID.to_string(), secrets)
        }
    };

    let mut guard = creds_cache().lock().await;
    *guard = Some(result.clone());
    result
}

/// Calcule la signature MD5 `request_sig` pour `track/getFileUrl`.
pub fn sign_get_file_url(track_id: &str, format_id: u32, request_ts: i64, app_secret: &str) -> String {
    let raw = format!("trackgetFileUrlformat_id{format_id}intentstreamtrack_id{track_id}{request_ts}{app_secret}");
    format!("{:x}", md5::compute(raw.as_bytes()))
}

fn form_body(params: &[(&str, &str)]) -> String {
    params
        .iter()
        .map(|(k, v)| format!("{}={}", urlencoding::encode(k), urlencoding::encode(v)))
        .collect::<Vec<_>>()
        .join("&")
}

async fn read_json(resp: Result<reqwest::Response, reqwest::Error>) -> Result<Value, String> {
    let resp = resp.map_err(|e| format!("Server unreachable: {e}"))?;
    let status = resp.status();
    let text = resp.text().await.map_err(|e| format!("Server unreachable: {e}"))?;
    let parsed: Option<Value> = serde_json::from_str(&text).ok();
    let api_msg = parsed
        .as_ref()
        .and_then(|v| v.get("message").and_then(Value::as_str))
        .map(str::trim)
        .filter(|s| !s.is_empty());

    if status == reqwest::StatusCode::UNAUTHORIZED {
        return Err(api_msg.unwrap_or("Qobuz email, password, or token rejected").into());
    }
    if status == reqwest::StatusCode::BAD_REQUEST {
        return Err(api_msg.unwrap_or("Invalid Qobuz request (check your credentials or token)").into());
    }
    if status == reqwest::StatusCode::NOT_FOUND {
        return Err("Item not found on Qobuz".into());
    }
    if !status.is_success() {
        return Err(match api_msg {
            Some(m) => format!("Qobuz: {m} (HTTP {})", status.as_u16()),
            None => format!("Qobuz: HTTP {}", status.as_u16()),
        });
    }
    parsed.ok_or_else(|| "Invalid Qobuz response".into())
}

fn info_from_user(user: &Value) -> ServerInfo {
    let sub = user
        .pointer("/credential/parameters/short_label")
        .or_else(|| user.pointer("/credential/label"))
        .or_else(|| user.pointer("/subscription/offer"))
        .and_then(Value::as_str)
        .unwrap_or("Hi-Res");
    let display = user
        .get("display_name")
        .or_else(|| user.get("login"))
        .or_else(|| user.get("email"))
        .and_then(Value::as_str)
        .map(|u| format!("Qobuz ({u})"));
    ServerInfo {
        kind: Some("qobuz".into()),
        name: display,
        device_id: None,
        version: Some(sub.to_string()),
        open_subsonic: false,
        extensions: vec!["bit-perfect-flac".into(), "hi-res-24-192".into()],
    }
}

impl Server {
    /// Connexion à Qobuz par e-mail/mot de passe ou par jeton `user_auth_token` (dans `api_key`).
    /// Dans `api_key`, l'utilisateur peut fournir soit `<user_auth_token>`, soit `<app_id>:<app_secret>:<user_auth_token>`.
    pub async fn connect(
        _raw_url: &str,
        username: &str,
        password: Option<&str>,
        api_key: Option<&str>,
    ) -> Result<(Server, ServerInfo), String> {
        let (discovered_app_id, discovered_secrets) = discover_app_credentials().await;

        if let Some(raw_key) = api_key.map(str::trim).filter(|k| !k.is_empty()) {
            let parts: Vec<&str> = raw_key.splitn(3, ':').collect();
            let (app_id, candidates, token) = match parts.as_slice() {
                [id, secret, tok] if !id.is_empty() && !secret.is_empty() && !tok.is_empty() => {
                    (id.trim().to_string(), vec![secret.trim().to_string()], tok.trim().to_string())
                }
                _ => (discovered_app_id, discovered_secrets, raw_key.to_string()),
            };
            return Self::login_with_token(app_id, candidates, token, username).await;
        }

        let user = username.trim();
        let pw = password.unwrap_or("").trim();
        if user.is_empty() || pw.is_empty() {
            return Err("Qobuz email/username and password are required".into());
        }

        Self::login_with_password(discovered_app_id, discovered_secrets, user, pw).await
    }

    async fn login_with_token(
        app_id: String,
        secrets: Vec<String>,
        token: String,
        fallback_username: &str,
    ) -> Result<(Server, ServerInfo), String> {
        let url = reqwest::Url::parse_with_params(
            &format!("{API_BASE}/favorite/getUserFavorites"),
            &[("type", "tracks"), ("limit", "1")],
        )
        .map_err(|e| e.to_string())?;
        let probe = read_json(
            client()
                .get(url)
                .header("X-App-Id", &app_id)
                .header("X-User-Auth-Token", &token)
                .timeout(Duration::from_secs(15))
                .send()
                .await,
        )
        .await?;

        let user_id = probe
            .pointer("/user/id")
            .and_then(Value::as_i64)
            .map(|n| n.to_string())
            .or_else(|| probe.pointer("/user/id").and_then(Value::as_str).map(str::to_string))
            .unwrap_or_else(|| {
                if fallback_username.trim().is_empty() {
                    "qobuz".to_string()
                } else {
                    fallback_username.trim().to_string()
                }
            });

        let app_secret = Self::pick_working_secret(&app_id, &token, &secrets).await;
        let server = Server {
            base: API_BASE.to_string(),
            app_id,
            app_secret,
            token,
            user_id,
        };
        let info = ServerInfo {
            kind: Some("qobuz".into()),
            name: Some(if fallback_username.trim().is_empty() {
                "Qobuz Hi-Res".into()
            } else {
                format!("Qobuz ({})", fallback_username.trim())
            }),
            device_id: None,
            version: Some("Hi-Res 24-bit / 192 kHz".into()),
            open_subsonic: false,
            extensions: vec!["bit-perfect-flac".into(), "hi-res-24-192".into()],
        };
        Ok((server, info))
    }

    async fn login_with_password(
        app_id: String,
        secrets: Vec<String>,
        user: &str,
        pw: &str,
    ) -> Result<(Server, ServerInfo), String> {
        let md5_pw = format!("{:x}", md5::compute(pw.as_bytes()));
        let is_already_md5 = pw.len() == 32 && pw.chars().all(|c| c.is_ascii_hexdigit());
        let pw_candidates = if is_already_md5 {
            vec![pw.to_ascii_lowercase(), md5_pw]
        } else {
            vec![md5_pw, pw.to_string()]
        };

        let user_field = if user.contains('@') { "email" } else { "username" };
        let mut last_err = String::from("Unable to sign in to Qobuz");

        for candidate_pw in &pw_candidates {
            for field in [user_field, if user_field == "email" { "username" } else { "email" }] {
                let params = [(field, user), ("password", candidate_pw.as_str()), ("app_id", app_id.as_str())];
                let Ok(url) = reqwest::Url::parse_with_params(&format!("{API_BASE}/user/login"), &params) else {
                    continue;
                };
                let body = form_body(&params);
                let resp = client()
                    .post(url)
                    .header("X-App-Id", &app_id)
                    .header("Content-Type", "application/x-www-form-urlencoded")
                    .body(body)
                    .timeout(Duration::from_secs(15))
                    .send()
                    .await;
                match read_json(resp).await {
                    Ok(json) => {
                        let Some(token) = json.get("user_auth_token").and_then(Value::as_str) else {
                            last_err = "Qobuz did not return a session token".into();
                            continue;
                        };
                        let user_obj = json.get("user").unwrap_or(&Value::Null);
                        let user_id = user_obj
                            .get("id")
                            .and_then(Value::as_i64)
                            .map(|id| id.to_string())
                            .or_else(|| user_obj.get("id").and_then(Value::as_str).map(str::to_string))
                            .unwrap_or_else(|| user.to_string());
                        let app_secret = Self::pick_working_secret(&app_id, token, &secrets).await;
                        let info = info_from_user(user_obj);
                        return Ok((
                            Server {
                                base: API_BASE.to_string(),
                                app_id,
                                app_secret,
                                token: token.to_string(),
                                user_id,
                            },
                            info,
                        ));
                    }
                    Err(e) => last_err = e,
                }
            }
        }
        Err(last_err)
    }

    /// Teste les `app_secret` candidats sur une requête signée `track/getFileUrl` afin de retenir celui accepté par Qobuz.
    async fn pick_working_secret(app_id: &str, token: &str, secrets: &[String]) -> String {
        let ts = chrono::Utc::now().timestamp();
        let ts_s = ts.to_string();
        for secret in secrets {
            let sig = sign_get_file_url("19512574", 5, ts, secret);
            let Ok(url) = reqwest::Url::parse_with_params(
                &format!("{API_BASE}/track/getFileUrl"),
                &[
                    ("track_id", "19512574"),
                    ("format_id", "5"),
                    ("intent", "stream"),
                    ("request_ts", ts_s.as_str()),
                    ("request_sig", sig.as_str()),
                ],
            ) else {
                continue;
            };
            let resp = client()
                .get(url)
                .header("X-App-Id", app_id)
                .header("X-User-Auth-Token", token)
                .timeout(Duration::from_secs(8))
                .send()
                .await;
            if let Ok(r) = resp {
                if r.status() != reqwest::StatusCode::BAD_REQUEST {
                    return secret.clone();
                }
            }
        }
        secrets.first().cloned().unwrap_or_else(|| DEFAULT_APP_SECRET.to_string())
    }

    pub(crate) async fn get(&self, path: &str, params: &[(&str, &str)]) -> Result<Value, String> {
        let url = reqwest::Url::parse_with_params(&format!("{}{path}", self.base), params).map_err(|e| e.to_string())?;
        read_json(
            client()
                .get(url)
                .header("X-App-Id", &self.app_id)
                .header("X-User-Auth-Token", &self.token)
                .timeout(Duration::from_secs(30))
                .send()
                .await,
        )
        .await
    }

    pub(crate) async fn post_form(&self, path: &str, params: &[(&str, &str)]) -> Result<Value, String> {
        let url = reqwest::Url::parse_with_params(&format!("{}{path}", self.base), params).map_err(|e| e.to_string())?;
        let body = form_body(params);
        read_json(
            client()
                .post(url)
                .header("X-App-Id", &self.app_id)
                .header("X-User-Auth-Token", &self.token)
                .header("Content-Type", "application/x-www-form-urlencoded")
                .body(body)
                .timeout(Duration::from_secs(30))
                .send()
                .await,
        )
        .await
    }

    /// Vérifie que la session Qobuz est active.
    pub async fn ping(&self) -> Result<ServerInfo, String> {
        let _ = self.get("/favorite/getUserFavorites", &[("type", "tracks"), ("limit", "1")]).await?;
        Ok(ServerInfo {
            kind: Some("qobuz".into()),
            name: Some("Qobuz".into()),
            device_id: None,
            version: Some("Hi-Res 24-bit / 192 kHz".into()),
            open_subsonic: false,
            extensions: vec!["bit-perfect-flac".into(), "hi-res-24-192".into()],
        })
    }

    /// Bibliothèques virtuelles proposées pour un compte Qobuz.
    pub async fn music_folders(&self) -> Result<Vec<MusicFolder>, String> {
        Ok(vec![
            MusicFolder {
                id: "favorites".into(),
                name: "Qobuz — Favorites (Albums & Tracks)".into(),
            },
            MusicFolder {
                id: "purchases".into(),
                name: "Qobuz — Purchases (Albums & Tracks)".into(),
            },
            MusicFolder {
                id: "featured".into(),
                name: "Qobuz — Hi-Res Featured Selection".into(),
            },
        ])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signs_track_file_url_with_md5() {
        let sig = sign_get_file_url("19512574", 27, 1700000000, "10b251c2868bf6d96c3f6f7fb0009093");
        assert_eq!(sig.len(), 32);
        assert!(sig.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn extracts_bundle_app_id_and_seed_secret() {
        // Secret de 32 octets : "10b251c2868bf6d96c3f6f7fb0009093" -> Base64 "MTBiMjUxYzI4NjhiZjZkOTZjM2Y2ZjdmYjAwMDkwOTM="
        // suivi de 44 caractères de remplissage ignorés.
        let b64 = "MTBiMjUxYzI4NjhiZjZkOTZjM2Y2ZjdmYjAwMDkwOTM=";
        let tail = "0123456789abcdef0123456789abcdef0123456789ab";
        assert_eq!(tail.len(), 44);
        let full = format!("{b64}{tail}");
        let seed = &full[..20];
        let info = &full[20..50];
        let extras = &full[50..];
        let mock_bundle = format!(
            r#"production:{{api:{{appId:"950096963",appSecret:""}}}};d.initialSeed("{seed}",window.utimezone.paris);timezones=[{{name:"Europe/Paris",info:"{info}",extras:"{extras}"}}]"#
        );
        let (app_id, secrets) = extract_bundle_credentials(&mock_bundle).unwrap();
        assert_eq!(app_id, "950096963");
        assert_eq!(secrets, vec!["10b251c2868bf6d96c3f6f7fb0009093"]);
    }
}
