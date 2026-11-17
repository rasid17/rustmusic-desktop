//! Serveurs de musique distants (OpenSubsonic, Jellyfin, DLNA) : ce qu'ils ont en commun, derrière [`Backend`].

use serde::{Deserialize, Serialize};

use crate::core::dlna_client::client::Server as Dlna;
use crate::core::jellyfin::client::Server as Jellyfin;
use crate::core::qobuz::client::{Server as Qobuz, API_BASE as QOBUZ_API_BASE, DEFAULT_APP_ID, DEFAULT_APP_SECRET};
use crate::core::tidal::client::{Server as Tidal, API_BASE as TIDAL_API_BASE};
use crate::core::tidal::catalog::TidalStream;
use crate::core::subsonic::client::{Auth, Server as Subsonic};
pub use crate::core::subsonic::catalog::RemoteSong;
pub use crate::core::subsonic::client::{MusicFolder, ServerInfo};
pub use crate::core::subsonic::lyrics::ServerLyrics;

/// Une playlist côté serveur, avec ses morceaux.
pub struct RemotePlaylist {
    pub id: String,
    pub name: String,
    pub comment: Option<String>,
    pub songs: Vec<RemoteSong>,
}

/// Préfixe des chemins de morceaux distants : `remote://<serveur>/<album>/<morceau>`.
pub const SCHEME: &str = "remote://";

/// Source d'un morceau : 0 pour « Cet ordinateur », sinon l'id de son serveur.
pub fn source_of(path: &str) -> i64 {
    path.strip_prefix(SCHEME).and_then(|r| r.split('/').next()?.parse().ok()).unwrap_or(0)
}

/// Bibliothèques d'une source (un paramètre), pour `… library_id IN (…)`.
pub const LIBRARIES_OF_SOURCE: &str = "SELECT id FROM library WHERE COALESCE(server_id, 0) = ?";

/// Condition « source active » sur une colonne `source_id` : un serveur désactivé est caché d'un bloc.
pub fn enabled_source(col: &str) -> String {
    format!("({col} = 0 OR {col} IN (SELECT id FROM media_servers WHERE enabled = 1))")
}

/// La sorte de serveur, gardée en base (`media_servers.kind`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Subsonic,
    Jellyfin,
    Dlna,
    Qobuz,
    Tidal,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Subsonic => "subsonic",
            Kind::Jellyfin => "jellyfin",
            Kind::Dlna => "dlna",
            Kind::Qobuz => "qobuz",
            Kind::Tidal => "tidal",
        }
    }

    pub fn parse(s: &str) -> Kind {
        match s.to_ascii_lowercase().as_str() {
            "jellyfin" => Kind::Jellyfin,
            "dlna" => Kind::Dlna,
            "qobuz" => Kind::Qobuz,
            "tidal" => Kind::Tidal,
            _ => Kind::Subsonic,
        }
    }
}

/// Ce qui se garde pour rejoindre le serveur (jamais le mot de passe).
#[derive(Debug, Default, PartialEq)]
pub struct Stored {
    pub token: Option<String>,
    pub salt: Option<String>,
    pub api_key: Option<String>,
    pub user_id: Option<String>,
}

/// Un serveur joint, quelle que soit sa sorte.
#[derive(Debug, Clone)]
pub enum Backend {
    Subsonic(Subsonic),
    Jellyfin(Jellyfin),
    Dlna(Dlna),
    Qobuz(Qobuz),
    Tidal(Tidal),
}

/// Source de téléchargement d'un morceau distant (URL simple ou séquence DASH fMP4).
pub enum RemoteStream {
    Url { url: reqwest::Url, ext: Option<&'static str> },
    Segments { urls: Vec<String>, ext: &'static str },
}

impl Backend {
    /// Joint le serveur : mot de passe (empreinte ou jeton de session) ou clé d'API.
    pub async fn connect(kind: Kind, url: &str, username: &str, password: Option<&str>, api_key: Option<&str>) -> Result<(Backend, ServerInfo), String> {
        let api_key = api_key.map(str::trim).filter(|k| !k.is_empty());
        match kind {
            Kind::Subsonic => {
                let auth = match (api_key, password.filter(|p| !p.is_empty())) {
                    (Some(key), _) => Auth::ApiKey(key.to_string()),
                    (None, Some(pw)) if !username.trim().is_empty() => Auth::from_password(username.trim(), pw),
                    _ => return Err("identifiant et mot de passe requis".into()),
                };
                let (s, info) = Subsonic::connect(url, auth).await?;
                Ok((Backend::Subsonic(s), info))
            }
            Kind::Jellyfin => {
                if username.trim().is_empty() {
                    return Err("nom d'utilisateur requis".into());
                }
                let (s, info) = Jellyfin::connect(url, username, password, api_key).await?;
                Ok((Backend::Jellyfin(s), info))
            }
            // DLNA : ouvert au réseau local, sans identifiants.
            Kind::Dlna => {
                let (s, info) = Dlna::connect(url).await?;
                Ok((Backend::Dlna(s), info))
            }
            Kind::Qobuz => {
                let (s, info) = Qobuz::connect(url, username, password, api_key).await?;
                Ok((Backend::Qobuz(s), info))
            }
            Kind::Tidal => {
                let t = Tidal::connect(url, username, password.unwrap_or(""), api_key).await?;
                let info = t.ping().await?;
                Ok((Backend::Tidal(t), info))
            }
        }
    }

    /// Le serveur enregistré, d'après ce qui a été gardé.
    pub fn restore(kind: Kind, url: String, username: String, stored: Stored) -> Result<Backend, String> {
        let missing = || "identifiants du serveur manquants".to_string();
        Ok(match kind {
            Kind::Subsonic => Backend::Subsonic(Subsonic {
                base: url,
                auth: match (stored.api_key, stored.token, stored.salt) {
                    (Some(key), _, _) => Auth::ApiKey(key),
                    (None, Some(token), Some(salt)) => Auth::Token { username, token, salt },
                    _ => return Err(missing()),
                },
            }),
            Kind::Jellyfin => {
                let user_id = stored.user_id.ok_or_else(missing)?;
                match (stored.api_key, stored.token) {
                    (Some(key), _) => Backend::Jellyfin(Jellyfin { base: url, token: key, user_id, api_key: true }),
                    (None, Some(token)) => Backend::Jellyfin(Jellyfin { base: url, token, user_id, api_key: false }),
                    _ => return Err(missing()),
                }
            }
            Kind::Dlna => Backend::Dlna(Dlna { location: url }),
            Kind::Qobuz => {
                let token = stored.token.filter(|t| !t.is_empty()).ok_or_else(missing)?;
                let app_id = stored.api_key.filter(|k| !k.is_empty()).unwrap_or_else(|| DEFAULT_APP_ID.to_string());
                let app_secret = stored.salt.filter(|s| !s.is_empty()).unwrap_or_else(|| DEFAULT_APP_SECRET.to_string());
                let user_id = stored.user_id.unwrap_or(username);
                let base = if url.is_empty() || url.starts_with("qobuz://") {
                    QOBUZ_API_BASE.to_string()
                } else {
                    url
                };
                Backend::Qobuz(Qobuz { base, app_id, app_secret, token, user_id })
            }
            Kind::Tidal => {
                let access = stored.token.filter(|t| !t.is_empty()).ok_or_else(missing)?;
                let refresh = stored.salt.unwrap_or_default();
                let api_key = stored.api_key.unwrap_or_default();
                let stored_uid = stored.user_id.unwrap_or_default();
                let base = if url.is_empty() || url.starts_with("tidal://") {
                    TIDAL_API_BASE.to_string()
                } else {
                    url
                };
                Backend::Tidal(Tidal::from_parts(&base, &username, &access, &refresh, &api_key, &stored_uid))
            }
        })
    }

    /// Ce qu'il faut garder pour le rejoindre plus tard.
    pub fn stored(&self) -> Stored {
        match self {
            Backend::Subsonic(s) => match &s.auth {
                Auth::Token { token, salt, .. } => Stored { token: Some(token.clone()), salt: Some(salt.clone()), ..Default::default() },
                Auth::ApiKey(key) => Stored { api_key: Some(key.clone()), ..Default::default() },
            },
            Backend::Jellyfin(j) => Stored {
                token: (!j.api_key).then(|| j.token.clone()),
                api_key: j.api_key.then(|| j.token.clone()),
                user_id: Some(j.user_id.clone()),
                ..Default::default()
            },
            Backend::Dlna(_) => Stored::default(),
            Backend::Qobuz(q) => Stored {
                token: Some(q.token.clone()),
                salt: Some(q.app_secret.clone()),
                api_key: Some(q.app_id.clone()),
                user_id: Some(q.user_id.clone()),
            },
            Backend::Tidal(t) => Stored {
                token: Some(t.effective_access_token()),
                salt: {
                    let r = t.effective_refresh_token();
                    (!r.is_empty()).then_some(r)
                },
                api_key: Some(t.api_key_string()),
                user_id: Some(t.stored_user_id()),
            },
        }
    }

    pub fn kind(&self) -> Kind {
        match self {
            Backend::Subsonic(_) => Kind::Subsonic,
            Backend::Jellyfin(_) => Kind::Jellyfin,
            Backend::Dlna(_) => Kind::Dlna,
            Backend::Qobuz(_) => Kind::Qobuz,
            Backend::Tidal(_) => Kind::Tidal,
        }
    }

    pub fn base(&self) -> &str {
        match self {
            Backend::Subsonic(s) => &s.base,
            Backend::Jellyfin(j) => &j.base,
            Backend::Dlna(d) => &d.location,
            Backend::Qobuz(q) => &q.base,
            Backend::Tidal(t) => &t.base,
        }
    }

    /// Le serveur tient des notes de 1 à 5 par utilisateur (Jellyfin, DLNA, Qobuz : non).
    pub fn has_ratings(&self) -> bool {
        matches!(self, Backend::Subsonic(_))
    }

    pub async fn ping(&self) -> Result<ServerInfo, String> {
        match self {
            Backend::Subsonic(s) => s.ping().await,
            Backend::Jellyfin(j) => j.ping().await,
            Backend::Dlna(d) => d.ping().await,
            Backend::Qobuz(q) => q.ping().await,
            Backend::Tidal(t) => t.ping().await,
        }
    }

    pub async fn music_folders(&self) -> Result<Vec<MusicFolder>, String> {
        match self {
            Backend::Subsonic(s) => s.music_folders().await,
            Backend::Jellyfin(j) => j.music_folders().await,
            Backend::Dlna(d) => Ok(d.folders("0").await?.into_iter().map(|f| MusicFolder { id: f.id, name: f.name }).collect()),
            Backend::Qobuz(q) => q.music_folders().await,
            Backend::Tidal(t) => t.music_folders().await,
        }
    }

    /// Les dossiers choisis, nommés : DLNA les prend n'importe où dans l'arborescence.
    pub async fn folders_by_id(&self, ids: &[String]) -> Result<Vec<MusicFolder>, String> {
        match self {
            Backend::Dlna(d) => {
                let mut out = Vec::new();
                for id in ids {
                    out.push(d.folder(id).await?);
                }
                Ok(out)
            }
            _ => Ok(self.music_folders().await?.into_iter().filter(|f| ids.contains(&f.id)).collect()),
        }
    }

    pub async fn all_songs(&self, folder_id: &str, progress: impl FnMut(usize)) -> Result<Vec<RemoteSong>, String> {
        match self {
            Backend::Subsonic(s) => s.all_songs(folder_id, progress).await,
            Backend::Jellyfin(j) => j.all_songs(folder_id, progress).await,
            Backend::Dlna(d) => d.all_songs(folder_id, progress).await,
            Backend::Qobuz(q) => q.all_songs(folder_id, progress).await,
            Backend::Tidal(t) => t.all_songs(folder_id, progress).await,
        }
    }

    pub async fn cover_art(&self, id: &str, size: u32) -> Option<Vec<u8>> {
        match self {
            Backend::Subsonic(s) => s.cover_art(id, size).await,
            Backend::Jellyfin(j) => j.cover_art(id, size).await,
            Backend::Dlna(d) => d.cover_art(id).await,
            Backend::Qobuz(q) => q.cover_art(id, size).await,
            Backend::Tidal(t) => t.cover_art(id, Some(size)).await.ok(),
        }
    }

    /// Adresse du flux : le fichier d'origine, ou transcodé (format, débit en kbit/s ; DLNA : jamais).
    pub async fn stream_url(&self, song_id: &str, transcode: Option<(&str, i64)>) -> Result<reqwest::Url, String> {
        match self {
            Backend::Subsonic(s) => {
                let bitrate = transcode.map(|(_, k)| k.to_string()).unwrap_or_default();
                let params: Vec<(&str, &str)> = match transcode {
                    Some((format, _)) => vec![("id", song_id), ("format", format), ("maxBitRate", &bitrate)],
                    None => vec![("id", song_id), ("format", "raw")],
                };
                s.url("stream", &params)
            }
            Backend::Jellyfin(j) => j.stream_url(song_id, transcode),
            Backend::Dlna(d) => d.stream_url(song_id).await,
            Backend::Qobuz(q) => q.stream_url(song_id, transcode).await,
            Backend::Tidal(t) => t.stream_url(song_id, transcode).await,
        }
    }

    /// Résout une cible de téléchargement complète (URL directe ou segmen DASH fMP4).
    pub async fn stream_target(&self, song_id: &str, transcode: Option<(&str, i64)>) -> Result<RemoteStream, String> {
        match self {
            Backend::Tidal(t) => match t.resolve_stream(song_id, transcode).await? {
                TidalStream::Direct { url, ext } => {
                    let parsed = reqwest::Url::parse(&url).map_err(|e| format!("Invalid TIDAL stream URL: {e}"))?;
                    Ok(RemoteStream::Url { url: parsed, ext: Some(ext) })
                }
                TidalStream::Segments { urls, ext } => Ok(RemoteStream::Segments { urls, ext }),
            },
            _ => {
                let url = self.stream_url(song_id, transcode).await?;
                Ok(RemoteStream::Url { url, ext: None })
            }
        }
    }

    /// Paroles du morceau ; `structured` : extension OpenSubsonic `songLyrics` annoncée.
    pub async fn lyrics(&self, song_id: &str, structured: bool, artist: &str, title: &str) -> Result<Option<ServerLyrics>, String> {
        match self {
            Backend::Subsonic(s) => s.lyrics(song_id, structured, artist, title).await,
            Backend::Jellyfin(j) => j.lyrics(song_id).await,
            Backend::Dlna(_) | Backend::Qobuz(_) | Backend::Tidal(_) => Ok(None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stored_credentials_round_trip() {
        let j = Backend::Jellyfin(Jellyfin { base: "http://nas:8096".into(), token: "tok".into(), user_id: "u1".into(), api_key: false });
        let back = Backend::restore(Kind::Jellyfin, "http://nas:8096".into(), "david".into(), j.stored()).unwrap();
        assert!(matches!(back, Backend::Jellyfin(ref x) if x.token == "tok" && x.user_id == "u1" && !x.api_key));
        let s = Backend::Subsonic(Subsonic { base: "http://nas:4533".into(), auth: Auth::ApiKey("k".into()) });
        assert!(matches!(Backend::restore(Kind::Subsonic, "x".into(), "d".into(), s.stored()).unwrap(), Backend::Subsonic(ref x) if x.auth == Auth::ApiKey("k".into())));
        assert!(Backend::restore(Kind::Jellyfin, "x".into(), "d".into(), Stored::default()).is_err());
        assert_eq!(Kind::parse("JellyFin"), Kind::Jellyfin);
        assert_eq!(Kind::parse("navidrome"), Kind::Subsonic);
        assert_eq!((source_of("remote://12/al/s"), source_of(r"C:\Musique\a.flac"), source_of("remote://x/a")), (12, 0, 0));
        assert!(matches!(Backend::restore(Kind::Dlna, "http://nas:8200/rootDesc.xml".into(), String::new(), Stored::default()).unwrap(), Backend::Dlna(_)));
    }

    #[tokio::test]
    async fn subsonic_stream_urls() {
        let s = Backend::Subsonic(Subsonic { base: "http://nas:4533".into(), auth: Auth::ApiKey("k".into()) });
        let raw = s.stream_url("s1", None).await.unwrap().to_string();
        assert!(raw.contains("/rest/stream?") && raw.contains("format=raw") && raw.contains("id=s1"));
        let mp3 = s.stream_url("s1", Some(("mp3", 192))).await.unwrap().to_string();
        assert!(mp3.contains("format=mp3") && mp3.contains("maxBitRate=192"));
    }
}
