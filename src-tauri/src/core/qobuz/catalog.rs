//! Catalogue Qobuz : favoris, achats, sélection audiophile, playlists, flux Hi-Res FLAC signés
//! et recherche directe dans le catalogue Qobuz.

use std::collections::HashSet;
use std::time::Duration;

use serde::Serialize;
use serde_json::Value;

use super::client::{discover_app_credentials, sign_get_file_url, Server};
use crate::core::podcast::fetch::client;
use crate::core::remote::RemotePlaylist;
use crate::core::subsonic::catalog::RemoteSong;

const PAGE_LIMIT: usize = 200;

/// Album trouvé lors d'une recherche dans le catalogue global Qobuz.
#[derive(Debug, Clone, Serialize)]
pub struct QobuzSearchAlbum {
    pub id: String,
    pub title: String,
    pub artist: String,
    pub year: Option<i64>,
    pub tracks_count: u32,
    pub duration: Option<f64>,
    pub bit_depth: Option<u32>,
    pub sampling_rate: Option<u32>,
    pub hires: bool,
    pub cover_url: Option<String>,
    pub genre: Option<String>,
}

/// Morceau trouvé lors d'une recherche dans le catalogue global Qobuz.
#[derive(Debug, Clone, Serialize)]
pub struct QobuzSearchTrack {
    pub id: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub album_id: String,
    pub year: Option<i64>,
    pub duration: Option<f64>,
    pub bit_depth: Option<u32>,
    pub sampling_rate: Option<u32>,
    pub hires: bool,
    pub cover_url: Option<String>,
}

/// Résultats d'une recherche dans le catalogue global Qobuz.
#[derive(Debug, Clone, Serialize)]
pub struct QobuzSearchResults {
    pub albums: Vec<QobuzSearchAlbum>,
    pub tracks: Vec<QobuzSearchTrack>,
}

fn id_str(v: Option<&Value>) -> Option<String> {
    let v = v?;
    if let Some(s) = v.as_str().map(str::trim).filter(|s| !s.is_empty()) {
        return Some(s.to_string());
    }
    if let Some(n) = v.as_i64() {
        return Some(n.to_string());
    }
    if let Some(u) = v.as_u64() {
        return Some(u.to_string());
    }
    None
}

fn text_of(v: &Value, key: &str) -> Option<String> {
    v.get(key).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()).map(str::to_string)
}

fn parse_year(album: &Value) -> Option<i64> {
    if let Some(date_str) = text_of(album, "release_date_original").or_else(|| text_of(album, "release_date_stream")) {
        if let Some(y) = date_str.get(0..4).and_then(|s| s.parse::<i64>().ok()) {
            return Some(y);
        }
    }
    if let Some(ts) = album.get("released_at").and_then(Value::as_i64) {
        if let Some(dt) = chrono::DateTime::from_timestamp(ts, 0) {
            return Some(chrono::Datelike::year(&dt) as i64);
        }
    }
    None
}

fn parse_sampling_rate_hz(v: Option<&Value>) -> Option<u32> {
    let raw = v.and_then(Value::as_f64).filter(|n| *n > 0.0)?;
    if raw <= 768.0 {
        Some((raw * 1000.0).round() as u32)
    } else {
        Some(raw.round() as u32)
    }
}

fn cover_from_album(album: &Value) -> Option<String> {
    album
        .pointer("/image/large")
        .or_else(|| album.pointer("/image/small"))
        .or_else(|| album.pointer("/image/thumbnail"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

pub(crate) fn parse_song(t: &Value, album_fallback: Option<&Value>, starred: bool) -> Option<RemoteSong> {
    let id = id_str(t.get("id"))?;
    let base_title = text_of(t, "title").unwrap_or_else(|| id.clone());
    let title = match text_of(t, "version") {
        Some(ver) if !base_title.to_lowercase().contains(&ver.to_lowercase()) => format!("{base_title} ({ver})"),
        _ => base_title,
    };

    let album_val = t.get("album").filter(|v| v.is_object()).or(album_fallback);
    let album_id = album_val.and_then(|a| id_str(a.get("id")));
    let album_title = album_val.and_then(|a| text_of(a, "title"));
    let album_artist = album_val.and_then(|a| {
        a.pointer("/artist/name")
            .or_else(|| a.pointer("/performer/name"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    });
    let artist = t
        .pointer("/performer/name")
        .or_else(|| t.pointer("/artist/name"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .or_else(|| album_artist.clone());

    let year = album_val.and_then(parse_year);
    let genre = album_val.and_then(|a| {
        a.pointer("/genre/name")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    });
    let cover_art = album_val.and_then(cover_from_album);

    let track = t.get("track_number").and_then(Value::as_u64).filter(|n| *n > 0).map(|n| n as u16);
    let disc = t.get("media_number").and_then(Value::as_u64).filter(|n| *n > 0).map(|n| n as u16);
    let duration = t.get("duration").and_then(Value::as_f64).filter(|d| *d > 0.0);

    let bit_depth = t
        .get("maximum_bit_depth")
        .or_else(|| album_val.and_then(|a| a.get("maximum_bit_depth")))
        .and_then(Value::as_u64)
        .filter(|b| *b > 0)
        .map(|b| b as u32)
        .or(Some(16));
    let sampling_rate = parse_sampling_rate_hz(
        t.get("maximum_sampling_rate")
            .or_else(|| album_val.and_then(|a| a.get("maximum_sampling_rate"))),
    )
    .or(Some(44_100));
    let channels = t
        .get("maximum_channel_count")
        .or_else(|| album_val.and_then(|a| a.get("maximum_channel_count")))
        .and_then(Value::as_u64)
        .filter(|c| *c > 0)
        .map(|c| c as u32)
        .or(Some(2));

    let bit_rate = match (bit_depth, sampling_rate, channels) {
        (Some(bd), Some(sr), Some(ch)) => Some(((sr as u64 * bd as u64 * ch as u64) / 1600) as u32),
        _ => Some(900),
    };

    Some(RemoteSong {
        id,
        title,
        album_id,
        artist,
        album: album_title,
        album_artist,
        year,
        genre,
        track,
        disc,
        duration,
        bit_rate,
        bit_depth,
        sampling_rate,
        channels,
        suffix: Some("flac".into()),
        size: None,
        cover_art,
        user_rating: None,
        starred,
        server_path: None,
    })
}

fn parse_search_album(a: &Value) -> Option<QobuzSearchAlbum> {
    let id = id_str(a.get("id"))?;
    let base_title = text_of(a, "title").unwrap_or_else(|| id.clone());
    let title = match text_of(a, "version") {
        Some(ver) if !base_title.to_lowercase().contains(&ver.to_lowercase()) => format!("{base_title} ({ver})"),
        _ => base_title,
    };
    let artist = a
        .pointer("/artist/name")
        .or_else(|| a.pointer("/performer/name"))
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or("Unknown Artist")
        .to_string();
    let bit_depth = a.get("maximum_bit_depth").and_then(Value::as_u64).filter(|b| *b > 0).map(|b| b as u32);
    let sampling_rate = parse_sampling_rate_hz(a.get("maximum_sampling_rate"));
    let hires = a.get("hires_streamable").and_then(Value::as_bool).unwrap_or_else(|| bit_depth.unwrap_or(16) > 16);
    Some(QobuzSearchAlbum {
        id,
        title,
        artist,
        year: parse_year(a),
        tracks_count: a.get("tracks_count").and_then(Value::as_u64).unwrap_or(0) as u32,
        duration: a.get("duration").and_then(Value::as_f64).filter(|d| *d > 0.0),
        bit_depth,
        sampling_rate,
        hires,
        cover_url: cover_from_album(a),
        genre: a.pointer("/genre/name").and_then(Value::as_str).map(str::to_string),
    })
}

fn parse_search_track(t: &Value) -> Option<QobuzSearchTrack> {
    let song = parse_song(t, None, false)?;
    let hires = t
        .get("hires_streamable")
        .and_then(Value::as_bool)
        .unwrap_or_else(|| song.bit_depth.unwrap_or(16) > 16);
    Some(QobuzSearchTrack {
        id: song.id,
        title: song.title,
        artist: song.artist.unwrap_or_else(|| "Unknown Artist".into()),
        album: song.album.unwrap_or_default(),
        album_id: song.album_id.unwrap_or_else(|| "_".into()),
        year: song.year,
        duration: song.duration,
        bit_depth: song.bit_depth,
        sampling_rate: song.sampling_rate,
        hires,
        cover_url: song.cover_art,
    })
}

/// Convertit le réglage de transcodage en `format_id` Qobuz :
/// - `None` (`raw`) -> `27` (FLAC Hi-Res 24 bits jusqu'à 192 kHz)
/// - `Some(("flac", 96))` -> `7` (FLAC Hi-Res 24 bits jusqu'à 96 kHz)
/// - `Some(("flac", 44))` -> `6` (FLAC Qualité CD 16 bits / 44,1 kHz)
/// - `Some(("mp3", _))` -> `5` (MP3 320 kbit/s)
pub fn format_id_of(transcode: Option<(&str, i64)>) -> u32 {
    match transcode {
        None => 27,
        Some(("flac", kbps)) if kbps >= 96 => 7,
        Some(("flac", _)) => 6,
        Some(("mp3", _)) => 5,
        Some(_) => 27,
    }
}

impl Server {
    /// Récupère tous les morceaux d'un album Qobuz (`/album/get`).
    pub async fn album_songs(&self, album_id: &str, starred: bool) -> Result<Vec<RemoteSong>, String> {
        let album = self.get("/album/get", &[("album_id", album_id), ("limit", "500")]).await?;
        let items = album.pointer("/tracks/items").and_then(Value::as_array).cloned().unwrap_or_default();
        Ok(items.iter().filter_map(|t| parse_song(t, Some(&album), starred)).collect())
    }

    /// Récupère un morceau Qobuz unique (`/track/get`).
    pub async fn track_song(&self, track_id: &str, starred: bool) -> Result<RemoteSong, String> {
        let track = self.get("/track/get", &[("track_id", track_id)]).await?;
        parse_song(&track, None, starred).ok_or_else(|| "Invalid Qobuz track".into())
    }

    /// Recherche globale d'albums et de titres dans le catalogue Qobuz.
    pub async fn search_catalog(&self, query: &str, limit: usize) -> Result<QobuzSearchResults, String> {
        let q = query.trim();
        if q.is_empty() {
            return Ok(QobuzSearchResults { albums: Vec::new(), tracks: Vec::new() });
        }
        let lim = limit.clamp(1, 50).to_string();
        let res = self.get("/catalog/search", &[("query", q), ("limit", &lim)]).await?;
        let albums = res
            .pointer("/albums/items")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(parse_search_album)
            .collect();
        let tracks = res
            .pointer("/tracks/items")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(parse_search_track)
            .collect();
        Ok(QobuzSearchResults { albums, tracks })
    }

    /// Tous les morceaux d'une bibliothèque virtuelle Qobuz (`favorites`, `purchases` ou `featured`).
    pub async fn all_songs(&self, folder_id: &str, mut progress: impl FnMut(usize)) -> Result<Vec<RemoteSong>, String> {
        let mut songs = Vec::new();
        let mut seen = HashSet::new();

        match folder_id {
            "purchases" => {
                let res = self.get("/purchase/getUserPurchases", &[("limit", "500")]).await?;
                for t in res.pointer("/tracks/items").and_then(Value::as_array).into_iter().flatten() {
                    if let Some(song) = parse_song(t, None, false) {
                        if seen.insert(song.id.clone()) {
                            songs.push(song);
                        }
                    }
                }
                progress(songs.len());
                let albums = res.pointer("/albums/items").and_then(Value::as_array).cloned().unwrap_or_default();
                for album in &albums {
                    if let Some(album_id) = id_str(album.get("id")) {
                        if let Ok(album_tracks) = self.album_songs(&album_id, false).await {
                            for song in album_tracks {
                                if seen.insert(song.id.clone()) {
                                    songs.push(song);
                                }
                            }
                            progress(songs.len());
                        }
                    }
                }
            }
            "featured" => {
                let res = self
                    .get("/album/getFeatured", &[("type", "ideal-discography"), ("limit", "25")])
                    .await?;
                let albums = res.pointer("/albums/items").and_then(Value::as_array).cloned().unwrap_or_default();
                for album in &albums {
                    if let Some(album_id) = id_str(album.get("id")) {
                        if let Ok(album_tracks) = self.album_songs(&album_id, false).await {
                            for song in album_tracks {
                                if seen.insert(song.id.clone()) {
                                    songs.push(song);
                                }
                            }
                            progress(songs.len());
                        }
                    }
                }
            }
            _ => {
                // "favorites" (par défaut) : titres favoris + tous les titres des albums favoris.
                let mut offset = 0usize;
                loop {
                    let off_s = offset.to_string();
                    let lim_s = PAGE_LIMIT.to_string();
                    let page = self
                        .get(
                            "/favorite/getUserFavorites",
                            &[("type", "tracks"), ("limit", &lim_s), ("offset", &off_s)],
                        )
                        .await?;
                    let items = page.pointer("/tracks/items").and_then(Value::as_array).cloned().unwrap_or_default();
                    let count = items.len();
                    for t in &items {
                        if let Some(song) = parse_song(t, None, true) {
                            if seen.insert(song.id.clone()) {
                                songs.push(song);
                            }
                        }
                    }
                    progress(songs.len());
                    let total = page.pointer("/tracks/total").and_then(Value::as_u64).unwrap_or(0) as usize;
                    offset += count;
                    if count < PAGE_LIMIT || offset >= total {
                        break;
                    }
                }

                let mut album_ids = Vec::new();
                let mut alb_offset = 0usize;
                loop {
                    let off_s = alb_offset.to_string();
                    let lim_s = PAGE_LIMIT.to_string();
                    let page = self
                        .get(
                            "/favorite/getUserFavorites",
                            &[("type", "albums"), ("limit", &lim_s), ("offset", &off_s)],
                        )
                        .await?;
                    let items = page.pointer("/albums/items").and_then(Value::as_array).cloned().unwrap_or_default();
                    let count = items.len();
                    for a in &items {
                        if let Some(id) = id_str(a.get("id")) {
                            album_ids.push(id);
                        }
                    }
                    let total = page.pointer("/albums/total").and_then(Value::as_u64).unwrap_or(0) as usize;
                    alb_offset += count;
                    if count < PAGE_LIMIT || alb_offset >= total {
                        break;
                    }
                }

                for album_id in album_ids {
                    if let Ok(album_tracks) = self.album_songs(&album_id, false).await {
                        for song in album_tracks {
                            if seen.insert(song.id.clone()) {
                                songs.push(song);
                            }
                        }
                        progress(songs.len());
                    }
                }
            }
        }

        Ok(songs)
    }

    /// Télécharge la pochette : `id` est l'URL directe de l'image Qobuz (ou un identifiant d'album).
    pub async fn cover_art(&self, id: &str, _size: u32) -> Option<Vec<u8>> {
        let image_url = if id.starts_with("http://") || id.starts_with("https://") {
            id.to_string()
        } else {
            let album = self.get("/album/get", &[("album_id", id)]).await.ok()?;
            cover_from_album(&album)?
        };
        let resp = client().get(&image_url).timeout(Duration::from_secs(20)).send().await.ok()?;
        if !resp.status().is_success() {
            return None;
        }
        resp.bytes().await.ok().map(|b| b.to_vec())
    }

    /// Demande à Qobuz l'URL de streaming signée (`track/getFileUrl`) pour le morceau et la qualité voulus.
    /// En cas de rotation du secret par Qobuz, réessaie automatiquement avec les secrets extraits du lecteur web.
    pub async fn stream_url(&self, song_id: &str, transcode: Option<(&str, i64)>) -> Result<reqwest::Url, String> {
        let preferred_format = format_id_of(transcode);
        let formats: Vec<u32> = match preferred_format {
            27 => vec![27, 7, 6, 5],
            7 => vec![7, 6, 5],
            6 => vec![6, 5],
            _ => vec![5],
        };

        let mut secrets = vec![self.app_secret.clone()];
        let (_, discovered) = discover_app_credentials().await;
        for s in discovered {
            if !secrets.contains(&s) {
                secrets.push(s);
            }
        }

        let mut last_err = String::from("Unable to obtain Qobuz stream URL");
        for secret in &secrets {
            for &fmt in &formats {
                let ts = chrono::Utc::now().timestamp();
                let sig = sign_get_file_url(song_id, fmt, ts, secret);
                let fmt_s = fmt.to_string();
                let ts_s = ts.to_string();
                match self
                    .get(
                        "/track/getFileUrl",
                        &[
                            ("track_id", song_id),
                            ("format_id", &fmt_s),
                            ("intent", "stream"),
                            ("request_ts", &ts_s),
                            ("request_sig", &sig),
                        ],
                    )
                    .await
                {
                    Ok(v) => {
                        if let Some(url_str) = v.get("url").and_then(Value::as_str).filter(|u| !u.is_empty()) {
                            if v.get("sample").and_then(Value::as_bool) == Some(true)
                                || v.pointer("/restrictions/0/code").and_then(Value::as_str) == Some("TrackRestrictedByRightHolders")
                            {
                                log::warn!("☁️  Qobuz: restricted sample for {song_id} (format {fmt})");
                            }
                            return reqwest::Url::parse(url_str).map_err(|e| e.to_string());
                        }
                        last_err = "Qobuz did not return a stream URL for this track (check subscription or rights)".into();
                    }
                    Err(e) => {
                        let invalid_sig = e.to_lowercase().contains("signature") || e.contains("HTTP 400");
                        last_err = e;
                        if invalid_sig {
                            // Ce `secret` est refusé : passer au secret suivant.
                            break;
                        }
                    }
                }
            }
        }
        Err(last_err)
    }

    /// Les playlists de l'utilisateur sur Qobuz.
    pub async fn playlists(&self) -> Result<Vec<RemotePlaylist>, String> {
        let list = self.get("/playlist/getUserPlaylists", &[("limit", "100")]).await?;
        let mut out = Vec::new();
        for p in list.pointer("/playlists/items").and_then(Value::as_array).into_iter().flatten() {
            let Some(id) = id_str(p.get("id")) else { continue };
            let detail = match self
                .get("/playlist/get", &[("playlist_id", &id), ("extra", "tracks"), ("limit", "500")])
                .await
            {
                Ok(d) => d,
                Err(_) => continue,
            };
            let songs = detail
                .pointer("/tracks/items")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|t| parse_song(t, None, false))
                .collect();
            out.push(RemotePlaylist {
                id: id.clone(),
                name: text_of(&detail, "name").or_else(|| text_of(p, "name")).unwrap_or(id),
                comment: text_of(&detail, "description").or_else(|| text_of(p, "description")),
                songs,
            });
        }
        Ok(out)
    }

    /// Crée une playlist privée sur le compte Qobuz de l'utilisateur.
    pub async fn create_playlist(&self, name: &str) -> Result<String, String> {
        let created = self
            .post_form(
                "/playlist/create",
                &[("name", name), ("is_public", "false"), ("is_collaborative", "false")],
            )
            .await?;
        id_str(created.get("id")).ok_or_else(|| "Qobuz did not return the created playlist ID".into())
    }

    /// Met à jour les métadonnées ou les morceaux d'une playlist Qobuz.
    pub async fn update_playlist(
        &self,
        id: &str,
        name: Option<&str>,
        description: Option<&str>,
        song_ids: Option<&[String]>,
    ) -> Result<(), String> {
        if name.is_some() || description.is_some() {
            let mut params = vec![("playlist_id", id)];
            if let Some(n) = name {
                params.push(("name", n));
            }
            if let Some(d) = description {
                params.push(("description", d));
            }
            let _ = self.post_form("/playlist/update", &params).await?;
        }
        if let Some(ids) = song_ids {
            // Supprime d'abord les anciens `playlist_track_id`, puis ajoute la nouvelle liste dans l'ordre.
            if let Ok(detail) = self.get("/playlist/get", &[("playlist_id", id), ("extra", "tracks"), ("limit", "500")]).await {
                let existing_pt_ids: Vec<String> = detail
                    .pointer("/tracks/items")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(|t| id_str(t.get("playlist_track_id")))
                    .collect();
                if !existing_pt_ids.is_empty() {
                    let joined = existing_pt_ids.join(",");
                    let _ = self
                        .post_form("/playlist/deleteTracks", &[("playlist_id", id), ("playlist_track_ids", &joined)])
                        .await;
                }
            }
            if !ids.is_empty() {
                let joined = ids.join(",");
                self.post_form(
                    "/playlist/addTracks",
                    &[("playlist_id", id), ("track_ids", &joined), ("no_duplicate", "false")],
                )
                .await?;
            }
        }
        Ok(())
    }

    pub async fn delete_playlist(&self, id: &str) -> Result<(), String> {
        self.post_form("/playlist/delete", &[("playlist_id", id)]).await.map(|_| ())
    }

    /// Ajoute ou retire un morceau des favoris Qobuz.
    pub async fn set_favorite(&self, song_id: &str, on: bool) -> Result<(), String> {
        let endpoint = if on { "/favorite/create" } else { "/favorite/delete" };
        self.post_form(endpoint, &[("track_ids", song_id)]).await.map(|_| ())
    }

    /// Ajoute un album aux favoris Qobuz.
    pub async fn set_favorite_album(&self, album_id: &str, on: bool) -> Result<(), String> {
        let endpoint = if on { "/favorite/create" } else { "/favorite/delete" };
        self.post_form(endpoint, &[("album_ids", album_id)]).await.map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_hires_qobuz_track() {
        let item = json!({
            "id": 19512574,
            "title": "Contact",
            "version": "Album Version",
            "track_number": 13,
            "media_number": 1,
            "duration": 381,
            "maximum_bit_depth": 24,
            "maximum_sampling_rate": 88.2,
            "maximum_channel_count": 2,
            "performer": { "name": "Daft Punk" },
            "album": {
                "id": "0886443919266",
                "title": "Random Access Memories",
                "release_date_original": "2013-05-17",
                "artist": { "name": "Daft Punk" },
                "genre": { "name": "Electronic" },
                "image": { "large": "https://static.qobuz.com/images/covers/66/92/0886443919266_600.jpg" }
            }
        });
        let song = parse_song(&item, None, true).unwrap();
        assert_eq!(song.id, "19512574");
        assert_eq!(song.title, "Contact (Album Version)");
        assert_eq!(song.artist.as_deref(), Some("Daft Punk"));
        assert_eq!(song.album.as_deref(), Some("Random Access Memories"));
        assert_eq!(song.album_id.as_deref(), Some("0886443919266"));
        assert_eq!(song.year, Some(2013));
        assert_eq!(song.bit_depth, Some(24));
        assert_eq!(song.sampling_rate, Some(88_200));
        assert_eq!(song.suffix.as_deref(), Some("flac"));
        assert!(song.starred);
    }

    #[test]
    fn maps_transcode_to_qobuz_format_id() {
        assert_eq!(format_id_of(None), 27);
        assert_eq!(format_id_of(Some(("flac", 96))), 7);
        assert_eq!(format_id_of(Some(("flac", 44))), 6);
        assert_eq!(format_id_of(Some(("mp3", 320))), 5);
    }
}
