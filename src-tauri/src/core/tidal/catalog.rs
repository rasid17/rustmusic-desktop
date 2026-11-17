//! Katalog, pencarian global, playlist, favorit, dan resolusi stream Bit-Perfect TIDAL.
//!
//! Mendukung dua jenis manifest dari `playbackinfopostpaywall`:
//! - `application/vnd.tidal.bts`: JSON berisi URL audio langsung (FLAC / AAC)
//! - `application/dash+xml`: XML MPD (*fragmented MP4* berisi stream `fLaC` 24-bit/192 kHz)

use std::collections::HashSet;

use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use reqwest::header::AUTHORIZATION;
use serde::Serialize;
use serde_json::Value;

use crate::core::podcast::fetch::client;
use crate::core::remote::RemotePlaylist;
use crate::core::subsonic::catalog::RemoteSong;

use super::client::{cover_art_url, json_id, Server};

const PAGE_SIZE: usize = 100;

/// Hasil resolusi pemutaran stream TIDAL.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TidalStream {
    /// Satu URL audio langsung (BTS JSON atau DASH `<BaseURL>`).
    Direct { url: String, ext: &'static str },
    /// Urutan segmen fMP4 DASH (`init` + segmen media berurutan) untuk didekode via `isomp4`.
    Segments { urls: Vec<String>, ext: &'static str },
}

impl TidalStream {
    pub fn ext(&self) -> &'static str {
        match self {
            TidalStream::Direct { ext, .. } | TidalStream::Segments { ext, .. } => ext,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TidalSearchAlbum {
    pub id: String,
    pub title: String,
    pub artist: String,
    pub year: Option<i64>,
    pub tracks_count: u32,
    pub cover_url: Option<String>,
    pub hires: bool,
    pub bit_depth: Option<u32>,
    pub sampling_rate: Option<u32>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TidalSearchTrack {
    pub id: String,
    pub album_id: Option<String>,
    pub title: String,
    pub artist: String,
    pub album: Option<String>,
    pub duration: Option<f64>,
    pub cover_url: Option<String>,
    pub hires: bool,
    pub bit_depth: Option<u32>,
    pub sampling_rate: Option<u32>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TidalSearchResults {
    pub albums: Vec<TidalSearchAlbum>,
    pub tracks: Vec<TidalSearchTrack>,
}

impl Server {
    /// Ambil daftar lagu untuk folder virtual (`favorites` atau `playlists`).
    pub async fn all_songs(
        &self,
        folder_id: &str,
        mut on_progress: impl FnMut(usize),
    ) -> Result<Vec<RemoteSong>, String> {
        let mut out = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();

        match folder_id {
            "playlists" | "playlists_tracks" => {
                let pls = self.playlists().await?;
                for pl in pls {
                    for song in pl.songs {
                        if seen.insert(song.id.clone()) {
                            out.push(song);
                            on_progress(out.len());
                        }
                    }
                }
            }
            _ => {
                // 1. Favorite tracks
                let fav_tracks = self
                    .fetch_paginated_items(
                        &format!("users/{}/favorites/tracks", self.user_id),
                        2000,
                    )
                    .await?;
                for item in &fav_tracks {
                    let raw = item.get("item").unwrap_or(item);
                    if let Some(mut song) = parse_track(raw, None) {
                        song.starred = true;
                        if seen.insert(song.id.clone()) {
                            out.push(song);
                            on_progress(out.len());
                        }
                    }
                }

                // 2. Favorite albums -> all tracks inside each album
                let fav_albums = self
                    .fetch_paginated_items(
                        &format!("users/{}/favorites/albums", self.user_id),
                        1000,
                    )
                    .await?;
                for item in &fav_albums {
                    let album = item.get("item").unwrap_or(item);
                    let Some(aid) = album.get("id").and_then(json_id) else {
                        continue;
                    };
                    if let Ok(tracks) = self.album_songs_with_meta(&aid, Some(album)).await {
                        for song in tracks {
                            if seen.insert(song.id.clone()) {
                                out.push(song);
                                on_progress(out.len());
                            }
                        }
                    }
                }
            }
        }

        Ok(out)
    }

    async fn fetch_paginated_items(
        &self,
        path: &str,
        max_items: usize,
    ) -> Result<Vec<Value>, String> {
        let mut all = Vec::new();
        let mut offset = 0usize;
        while all.len() < max_items {
            let limit_s = PAGE_SIZE.to_string();
            let offset_s = offset.to_string();
            let val = self
                .get_json(path, &[("limit", &limit_s), ("offset", &offset_s)])
                .await?;
            let items = val
                .get("items")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            let batch_len = items.len();
            all.extend(items);
            let total = val
                .get("totalNumberOfItems")
                .and_then(Value::as_u64)
                .map(|n| n as usize)
                .unwrap_or(all.len());
            if batch_len == 0 || all.len() >= total || batch_len < PAGE_SIZE {
                break;
            }
            offset += batch_len;
        }
        Ok(all)
    }

    pub async fn album_songs(&self, album_id: &str) -> Result<Vec<RemoteSong>, String> {
        let album = self.get_json(&format!("albums/{album_id}"), &[]).await.ok();
        self.album_songs_with_meta(album_id, album.as_ref()).await
    }

    async fn album_songs_with_meta(
        &self,
        album_id: &str,
        album_fallback: Option<&Value>,
    ) -> Result<Vec<RemoteSong>, String> {
        let val = self
            .get_json(
                &format!("albums/{album_id}/tracks"),
                &[("limit", "500"), ("offset", "0")],
            )
            .await?;
        let items = val
            .get("items")
            .and_then(Value::as_array)
            .ok_or("Empty album track list from TIDAL")?;
        let mut songs = Vec::with_capacity(items.len());
        for item in items {
            let raw = item.get("item").unwrap_or(item);
            if let Some(song) = parse_track(raw, album_fallback) {
                songs.push(song);
            }
        }
        Ok(songs)
    }

    pub async fn track_song(&self, track_id: &str) -> Result<RemoteSong, String> {
        let val = self.get_json(&format!("tracks/{track_id}"), &[]).await?;
        parse_track(&val, None).ok_or_else(|| "Invalid TIDAL track metadata".to_string())
    }

    pub async fn search_catalog(
        &self,
        query: &str,
        limit: usize,
    ) -> Result<TidalSearchResults, String> {
        let trimmed = query.trim();
        if trimmed.is_empty() {
            return Ok(TidalSearchResults {
                albums: Vec::new(),
                tracks: Vec::new(),
            });
        }
        let limit_s = limit.clamp(1, 50).to_string();
        let val = self
            .get_json(
                "search",
                &[
                    ("query", trimmed),
                    ("types", "ALBUMS,TRACKS"),
                    ("limit", &limit_s),
                ],
            )
            .await?;

        let albums = val
            .pointer("/albums/items")
            .and_then(Value::as_array)
            .map(|arr| arr.iter().filter_map(parse_search_album).collect())
            .unwrap_or_default();

        let tracks = val
            .pointer("/tracks/items")
            .and_then(Value::as_array)
            .map(|arr| arr.iter().filter_map(parse_search_track).collect())
            .unwrap_or_default();

        Ok(TidalSearchResults { albums, tracks })
    }

    /// Resolusi URL/segmen audio bit-perfect melalui `playbackinfopostpaywall`.
    pub async fn resolve_stream(
        &self,
        song_id: &str,
        transcode: Option<(&str, i64)>,
    ) -> Result<TidalStream, String> {
        let qualities: &[&str] = match transcode {
            None => &["HI_RES_LOSSLESS", "LOSSLESS", "HIGH"],
            Some(("flac", k)) if k < 96 => &["LOSSLESS", "HIGH"],
            Some(("flac", _)) => &["HI_RES_LOSSLESS", "LOSSLESS", "HIGH"],
            Some(("aac" | "m4a" | "mp3" | "opus", k)) if k <= 96 => &["LOW", "HIGH"],
            Some(_) => &["HIGH"],
        };

        let mut last_err = String::new();
        for &quality in qualities {
            match self
                .get_json(
                    &format!("tracks/{song_id}/playbackinfopostpaywall"),
                    &[
                        ("audioquality", quality),
                        ("playbackmode", "STREAM"),
                        ("assetpresentation", "FULL"),
                    ],
                )
                .await
            {
                Ok(val) => {
                    let mime = val
                        .get("manifestMimeType")
                        .and_then(Value::as_str)
                        .unwrap_or("");
                    let b64 = val.get("manifest").and_then(Value::as_str).unwrap_or("");
                    if b64.is_empty() {
                        last_err = format!("Empty manifest for TIDAL quality {quality}");
                        continue;
                    }
                    match parse_manifest(mime, b64) {
                        Ok(stream) => return Ok(stream),
                        Err(e) => {
                            last_err = e;
                            continue;
                        }
                    }
                }
                Err(e) => {
                    last_err = e;
                }
            }
        }
        Err(if last_err.is_empty() {
            "No playable TIDAL stream found for this track".into()
        } else {
            last_err
        })
    }

    pub async fn stream_url(
        &self,
        song_id: &str,
        transcode: Option<(&str, i64)>,
    ) -> Result<reqwest::Url, String> {
        match self.resolve_stream(song_id, transcode).await? {
            TidalStream::Direct { url, .. } => {
                reqwest::Url::parse(&url).map_err(|e| format!("Invalid TIDAL stream URL: {e}"))
            }
            TidalStream::Segments { urls, .. } => {
                let first = urls.first().ok_or("Empty DASH segment list")?;
                reqwest::Url::parse(first).map_err(|e| format!("Invalid TIDAL segment URL: {e}"))
            }
        }
    }

    pub async fn cover_art(
        &self,
        cover_id: &str,
        size: Option<u32>,
    ) -> Result<Vec<u8>, String> {
        let Some(url) = cover_art_url(cover_id, size) else {
            return Err("Empty TIDAL cover ID".into());
        };
        let resp = client()
            .get(&url)
            .send()
            .await
            .map_err(|e| format!("TIDAL cover fetch failed: {e}"))?;
        if !resp.status().is_success() {
            return Err(format!("TIDAL cover HTTP {}", resp.status().as_u16()));
        }
        resp.bytes()
            .await
            .map(|b| b.to_vec())
            .map_err(|e| e.to_string())
    }

    pub async fn playlists(&self) -> Result<Vec<RemotePlaylist>, String> {
        let items = self
            .fetch_paginated_items(&format!("users/{}/playlists", self.user_id), 250)
            .await?;
        let mut out = Vec::with_capacity(items.len());
        for pl in items {
            let Some(uuid) = pl.get("uuid").and_then(Value::as_str) else {
                continue;
            };
            let name = pl
                .get("title")
                .and_then(Value::as_str)
                .unwrap_or("Playlist")
                .to_string();
            let comment = pl
                .get("description")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string);
            let tracks = self
                .fetch_paginated_items(&format!("playlists/{uuid}/tracks"), 2000)
                .await
                .unwrap_or_default();
            let songs: Vec<RemoteSong> = tracks
                .iter()
                .filter_map(|item| {
                    let raw = item.get("item").unwrap_or(item);
                    parse_track(raw, None)
                })
                .collect();
            out.push(RemotePlaylist {
                id: uuid.to_string(),
                name,
                comment,
                songs,
            });
        }
        Ok(out)
    }

    async fn playlist_etag(&self, uuid: &str) -> String {
        let Ok(url) = self.build_url(&format!("playlists/{uuid}"), &[]) else {
            return "*".into();
        };
        let token = self.effective_access_token();
        if let Ok(resp) = client()
            .get(url)
            .header(AUTHORIZATION, format!("Bearer {token}"))
            .send()
            .await
        {
            if let Some(etag) = resp.headers().get("ETag").and_then(|v| v.to_str().ok()) {
                return etag.to_string();
            }
        }
        "*".into()
    }

    pub async fn create_playlist(&self, name: &str, songs: &[&str]) -> Result<String, String> {
        let (val, _) = self
            .post_form(
                &format!("users/{}/playlists", self.user_id),
                &[],
                &[("title", name), ("description", "")],
                None,
            )
            .await?;
        let uuid = val
            .get("uuid")
            .and_then(Value::as_str)
            .ok_or("Missing playlist uuid from TIDAL")?
            .to_string();
        if !songs.is_empty() {
            let _ = self.add_to_playlist(&uuid, songs).await;
        }
        Ok(uuid)
    }

    pub async fn rename_playlist(&self, uuid: &str, name: &str) -> Result<(), String> {
        let etag = self.playlist_etag(uuid).await;
        self.post_form(
            &format!("playlists/{uuid}"),
            &[],
            &[("title", name)],
            Some(&etag),
        )
        .await?;
        Ok(())
    }

    pub async fn delete_playlist(&self, uuid: &str) -> Result<(), String> {
        let etag = self.playlist_etag(uuid).await;
        self.delete_req(&format!("playlists/{uuid}"), &[], Some(&etag))
            .await
    }

    pub async fn add_to_playlist(&self, uuid: &str, songs: &[&str]) -> Result<(), String> {
        if songs.is_empty() {
            return Ok(());
        }
        let etag = self.playlist_etag(uuid).await;
        let joined = songs.join(",");
        self.post_form(
            &format!("playlists/{uuid}/items"),
            &[],
            &[("trackIds", &joined), ("onDupes", "SKIP")],
            Some(&etag),
        )
        .await?;
        Ok(())
    }

    pub async fn remove_from_playlist(&self, uuid: &str, indices: &[usize]) -> Result<(), String> {
        if indices.is_empty() {
            return Ok(());
        }
        let etag = self.playlist_etag(uuid).await;
        let idx_str = indices
            .iter()
            .map(|i| i.to_string())
            .collect::<Vec<_>>()
            .join(",");
        self.delete_req(
            &format!("playlists/{uuid}/items/{idx_str}"),
            &[],
            Some(&etag),
        )
        .await
    }

    /// Sinkronkan isi playlist TIDAL agar sesuai daftar ID track lokal.
    pub async fn sync_playlist_tracks(&self, uuid: &str, desired_ids: &[String]) -> Result<(), String> {
        let items = self
            .fetch_paginated_items(&format!("playlists/{uuid}/tracks"), 2000)
            .await?;
        let current: Vec<String> = items
            .iter()
            .filter_map(|item| item.get("item").unwrap_or(item).get("id").and_then(json_id))
            .collect();
        let desired_set: HashSet<&str> = desired_ids.iter().map(String::as_str).collect();
        let current_set: HashSet<&str> = current.iter().map(String::as_str).collect();

        let to_remove: Vec<usize> = current
            .iter()
            .enumerate()
            .filter(|(_, id)| !desired_set.contains(id.as_str()))
            .map(|(i, _)| i)
            .collect();
        if !to_remove.is_empty() {
            self.remove_from_playlist(uuid, &to_remove).await?;
        }

        let to_add: Vec<&str> = desired_ids
            .iter()
            .map(String::as_str)
            .filter(|id| !current_set.contains(*id))
            .collect();
        if !to_add.is_empty() {
            self.add_to_playlist(uuid, &to_add).await?;
        }
        Ok(())
    }

    pub async fn set_favorite(&self, track_id: &str, starred: bool) -> Result<(), String> {
        if starred {
            self.post_form(
                &format!("users/{}/favorites/tracks", self.user_id),
                &[],
                &[("trackIds", track_id), ("onArtifactNotFound", "SKIP")],
                None,
            )
            .await?;
        } else {
            self.delete_req(
                &format!("users/{}/favorites/tracks/{track_id}", self.user_id),
                &[],
                None,
            )
            .await?;
        }
        Ok(())
    }

    pub async fn set_favorite_album(&self, album_id: &str, starred: bool) -> Result<(), String> {
        if starred {
            self.post_form(
                &format!("users/{}/favorites/albums", self.user_id),
                &[],
                &[("albumIds", album_id), ("onArtifactNotFound", "SKIP")],
                None,
            )
            .await?;
        } else {
            self.delete_req(
                &format!("users/{}/favorites/albums/{album_id}", self.user_id),
                &[],
                None,
            )
            .await?;
        }
        Ok(())
    }
}

/// Decode manifest Base64 dari `playbackinfopostpaywall` (BTS JSON atau DASH XML).
pub(crate) fn parse_manifest(mime_type: &str, manifest_b64: &str) -> Result<TidalStream, String> {
    let raw = STANDARD
        .decode(manifest_b64.trim())
        .map_err(|e| format!("Invalid TIDAL manifest base64: {e}"))?;
    let text = String::from_utf8(raw)
        .map_err(|e| format!("Invalid UTF-8 in TIDAL manifest: {e}"))?;

    if mime_type.eq_ignore_ascii_case("application/dash+xml")
        || text.trim_start().starts_with('<')
    {
        return parse_dash_mpd(&text);
    }

    // application/vnd.tidal.bts
    let v: Value = serde_json::from_str(&text)
        .map_err(|e| format!("Invalid TIDAL BTS manifest JSON: {e}"))?;
    let enc = v
        .get("encryptionType")
        .and_then(Value::as_str)
        .unwrap_or("NONE");
    if !enc.eq_ignore_ascii_case("NONE") && !enc.is_empty() {
        return Err(format!("Unsupported TIDAL stream encryption: {enc}"));
    }
    let url = v
        .pointer("/urls/0")
        .and_then(Value::as_str)
        .ok_or("No audio URL in TIDAL manifest")?
        .to_string();
    let mime = v.get("mimeType").and_then(Value::as_str).unwrap_or("");
    let codecs = v.get("codecs").and_then(Value::as_str).unwrap_or("");
    let ext = if mime.contains("flac") || codecs.contains("flac") {
        "flac"
    } else if mime.contains("mp4") || codecs.contains("mp4a") {
        "m4a"
    } else {
        "flac"
    };
    Ok(TidalStream::Direct { url, ext })
}

/// Parser XML MPD untuk manifest DASH TIDAL (`<BaseURL>` atau `<SegmentTemplate>` + `<SegmentTimeline>`).
pub(crate) fn parse_dash_mpd(xml: &str) -> Result<TidalStream, String> {
    let rep_block = select_best_audio_block(xml).unwrap_or(xml);

    if let Some(seg_tpl_tag) = extract_opening_tag(rep_block, "SegmentTemplate")
        .or_else(|| extract_opening_tag(xml, "SegmentTemplate"))
    {
        let init = xml_attr(seg_tpl_tag, "initialization")
            .ok_or("Missing initialization attribute in TIDAL DASH MPD")?;
        let media = xml_attr(seg_tpl_tag, "media")
            .ok_or("Missing media attribute in TIDAL DASH MPD")?;
        let start_number: u64 = xml_attr(seg_tpl_tag, "startNumber")
            .and_then(|s| s.parse().ok())
            .unwrap_or(1);
        let rep_id = extract_opening_tag(rep_block, "Representation")
            .and_then(|t| xml_attr(t, "id"))
            .unwrap_or_default();

        let timeline_src = extract_element_body(rep_block, "SegmentTimeline")
            .or_else(|| extract_element_body(xml, "SegmentTimeline"))
            .unwrap_or(rep_block);
        let segment_count = count_timeline_segments(timeline_src);
        if segment_count == 0 {
            return Err("Empty SegmentTimeline in TIDAL DASH MPD".into());
        }

        let base_url = extract_element_body(rep_block, "BaseURL")
            .or_else(|| extract_element_body(xml, "BaseURL"))
            .map(unescape_xml);

        let resolve_tpl = |tpl: String| -> String {
            let unescaped = unescape_xml(&tpl).replace("$RepresentationID$", &rep_id);
            if unescaped.starts_with("http://") || unescaped.starts_with("https://") {
                unescaped
            } else if let Some(ref b) = base_url {
                format!("{}{}", b.trim_end_matches('/'), if unescaped.starts_with('/') { unescaped } else { format!("/{unescaped}") })
            } else {
                unescaped
            }
        };

        let init_url = resolve_tpl(init);
        let media_tpl = resolve_tpl(media);

        let mut urls = Vec::with_capacity(1 + segment_count);
        urls.push(init_url);
        for idx in 0..segment_count {
            let num = (start_number + idx as u64).to_string();
            urls.push(media_tpl.replace("$Number$", &num));
        }
        return Ok(TidalStream::Segments { urls, ext: "mp4" });
    }

    if let Some(base) = extract_element_body(rep_block, "BaseURL")
        .or_else(|| extract_element_body(xml, "BaseURL"))
    {
        return Ok(TidalStream::Direct {
            url: unescape_xml(base).trim().to_string(),
            ext: "mp4",
        });
    }

    Err("Unsupported TIDAL DASH MPD layout".into())
}

fn select_best_audio_block(xml: &str) -> Option<&str> {
    let mut best: Option<(u64, &str)> = None;
    let mut pos = 0;
    while let Some(rel_start) = xml[pos..].find("<Representation") {
        let start = pos + rel_start;
        let rest = &xml[start..];
        let tag_end = rest.find('>')?;
        let tag = &rest[..=tag_end];
        let bw: u64 = xml_attr(tag, "bandwidth")
            .and_then(|b| b.parse().ok())
            .unwrap_or(0);
        let codecs = xml_attr(tag, "codecs").unwrap_or_default();
        let score = if codecs.contains("flac") { bw + 1_000_000_000 } else { bw };
        let block_end = rest.find("</Representation>").map(|e| start + e + 17).unwrap_or(xml.len());
        let block = &xml[start..block_end];
        if best.map_or(true, |(cur, _)| score > cur) {
            best = Some((score, block));
        }
        pos = block_end;
    }
    best.map(|(_, b)| b)
}

fn extract_opening_tag<'a>(xml: &'a str, tag_name: &str) -> Option<&'a str> {
    let needle = format!("<{tag_name}");
    let start = xml.find(&needle)?;
    let rest = &xml[start..];
    let after = rest.as_bytes().get(needle.len()).copied()?;
    if !matches!(after, b' ' | b'>' | b'/' | b'\t' | b'\r' | b'\n') {
        return None;
    }
    let end = rest.find('>')?;
    Some(&rest[..=end])
}

fn extract_element_body<'a>(xml: &'a str, tag_name: &str) -> Option<&'a str> {
    let open = format!("<{tag_name}");
    let start = xml.find(&open)?;
    let rest = &xml[start..];
    let open_end = rest.find('>')?;
    if rest[..open_end].ends_with('/') {
        return None;
    }
    let body = &rest[open_end + 1..];
    let close = format!("</{tag_name}>");
    let end = body.find(&close)?;
    Some(body[..end].trim())
}

fn xml_attr(tag: &str, name: &str) -> Option<String> {
    for quote in ['"', '\''] {
        let pat = format!("{name}={quote}");
        if let Some(pos) = tag.find(&pat) {
            let after = &tag[pos + pat.len()..];
            if let Some(end) = after.find(quote) {
                return Some(after[..end].to_string());
            }
        }
    }
    None
}

fn count_timeline_segments(timeline_xml: &str) -> usize {
    let mut total = 0usize;
    let mut pos = 0usize;
    while let Some(rel) = timeline_xml[pos..].find("<S") {
        let idx = pos + rel;
        let next_byte = timeline_xml.as_bytes().get(idx + 2).copied();
        if !matches!(next_byte, Some(b' ' | b'/' | b'>' | b'\t')) {
            pos = idx + 2;
            continue;
        }
        let Some(end_rel) = timeline_xml[idx..].find('>') else {
            break;
        };
        let tag = &timeline_xml[idx..=idx + end_rel];
        let repeat = xml_attr(tag, "r")
            .and_then(|s| s.parse::<i64>().ok())
            .filter(|&r| r >= 0)
            .unwrap_or(0) as usize;
        total += 1 + repeat;
        pos = idx + end_rel + 1;
    }
    total
}

fn unescape_xml(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
}

fn parse_track(t: &Value, album_fallback: Option<&Value>) -> Option<RemoteSong> {
    let id = t.get("id").and_then(json_id)?;
    let raw_title = t.get("title").and_then(Value::as_str)?.trim();
    if raw_title.is_empty() {
        return None;
    }
    let version = t
        .get("version")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let title = match version {
        Some(v) if !raw_title.contains(v) => format!("{raw_title} ({v})"),
        _ => raw_title.to_string(),
    };

    let album_obj = t.get("album").filter(|a| !a.is_null()).or(album_fallback);
    let album_id = album_obj.and_then(|a| a.get("id")).and_then(json_id);
    let album_title = album_obj
        .and_then(|a| a.get("title"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let cover_art = album_obj
        .and_then(|a| a.get("cover"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let release_date = album_obj
        .and_then(|a| a.get("releaseDate"))
        .and_then(Value::as_str)
        .or_else(|| t.get("streamStartDate").and_then(Value::as_str));
    let year = release_date
        .and_then(|d| d.get(..4))
        .and_then(|y| y.parse::<i64>().ok());

    let artist = parse_artists(t)
        .or_else(|| album_obj.and_then(parse_artists));
    let album_artist = album_obj
        .and_then(parse_artists)
        .or_else(|| artist.clone());

    let quality = t
        .get("audioQuality")
        .and_then(Value::as_str)
        .unwrap_or("LOSSLESS");
    let has_hires_tag = quality.eq_ignore_ascii_case("HI_RES_LOSSLESS")
        || quality.eq_ignore_ascii_case("HI_RES")
        || t.pointer("/mediaMetadata/tags")
            .and_then(Value::as_array)
            .map_or(false, |tags| {
                tags.iter()
                    .filter_map(Value::as_str)
                    .any(|tag| tag.eq_ignore_ascii_case("HIRES_LOSSLESS"))
            });

    let (bit_depth, sampling_rate, bit_rate, suffix) = if has_hires_tag {
        (Some(24), Some(96000), Some(3200), Some("flac".to_string()))
    } else if quality.eq_ignore_ascii_case("LOSSLESS") {
        (Some(16), Some(44100), Some(1411), Some("flac".to_string()))
    } else {
        (Some(16), Some(44100), Some(320), Some("m4a".to_string()))
    };

    Some(RemoteSong {
        id,
        album_id,
        title,
        artist,
        album: album_title,
        album_artist,
        year,
        genre: None,
        track: t
            .get("trackNumber")
            .and_then(Value::as_u64)
            .filter(|&n| n > 0)
            .map(|n| n as u16),
        disc: t
            .get("volumeNumber")
            .and_then(Value::as_u64)
            .filter(|&n| n > 0)
            .map(|n| n as u16),
        duration: t.get("duration").and_then(Value::as_f64).filter(|&d| d > 0.0),
        bit_rate,
        bit_depth,
        sampling_rate,
        channels: Some(2),
        suffix,
        size: None,
        cover_art,
        user_rating: None,
        starred: false,
        server_path: None,
    })
}

fn parse_artists(obj: &Value) -> Option<String> {
    if let Some(arr) = obj.get("artists").and_then(Value::as_array) {
        let names: Vec<&str> = arr
            .iter()
            .filter_map(|a| a.get("name").and_then(Value::as_str))
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .collect();
        if !names.is_empty() {
            return Some(names.join("; "));
        }
    }
    obj.pointer("/artist/name")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

fn parse_search_album(a: &Value) -> Option<TidalSearchAlbum> {
    let id = a.get("id").and_then(json_id)?;
    let title = a.get("title").and_then(Value::as_str)?.trim().to_string();
    if title.is_empty() {
        return None;
    }
    let artist = parse_artists(a).unwrap_or_else(|| "Unknown Artist".into());
    let year = a
        .get("releaseDate")
        .and_then(Value::as_str)
        .and_then(|d| d.get(..4))
        .and_then(|y| y.parse::<i64>().ok());
    let cover_url = a
        .get("cover")
        .and_then(Value::as_str)
        .and_then(|c| cover_art_url(c, Some(640)));
    let hires = a
        .get("audioQuality")
        .and_then(Value::as_str)
        .map_or(false, |q| q.contains("HI_RES"))
        || a.pointer("/mediaMetadata/tags")
            .and_then(Value::as_array)
            .map_or(false, |tags| {
                tags.iter()
                    .filter_map(Value::as_str)
                    .any(|t| t.eq_ignore_ascii_case("HIRES_LOSSLESS"))
            });
    Some(TidalSearchAlbum {
        id,
        title,
        artist,
        year,
        tracks_count: a
            .get("numberOfTracks")
            .and_then(Value::as_u64)
            .unwrap_or(0) as u32,
        cover_url,
        hires,
        bit_depth: Some(if hires { 24 } else { 16 }),
        sampling_rate: Some(if hires { 96000 } else { 44100 }),
    })
}

fn parse_search_track(t: &Value) -> Option<TidalSearchTrack> {
    let song = parse_track(t, None)?;
    let cover_url = song
        .cover_art
        .as_deref()
        .and_then(|c| cover_art_url(c, Some(320)));
    let hires = song.bit_depth.unwrap_or(16) >= 24;
    Some(TidalSearchTrack {
        id: song.id,
        album_id: song.album_id,
        title: song.title,
        artist: song.artist.unwrap_or_else(|| "Unknown Artist".into()),
        album: song.album,
        duration: song.duration,
        cover_url,
        hires,
        bit_depth: song.bit_depth,
        sampling_rate: song.sampling_rate,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_bts_flac_manifest() {
        let json = r#"{
            "mimeType": "audio/flac",
            "codecs": "flac",
            "encryptionType": "NONE",
            "urls": ["https://audio.tidal.com/track-123.flac?token=abc"]
        }"#;
        let encoded = STANDARD.encode(json.as_bytes());
        let stream = parse_manifest("application/vnd.tidal.bts", &encoded).unwrap();
        assert_eq!(
            stream,
            TidalStream::Direct {
                url: "https://audio.tidal.com/track-123.flac?token=abc".into(),
                ext: "flac",
            }
        );
    }

    #[test]
    fn parses_dash_mpd_segmented_hires_flac() {
        let mpd = r#"<?xml version='1.0' encoding='UTF-8'?>
        <MPD xmlns="urn:mpeg:dash:schema:mpd:2011">
          <Period>
            <AdaptationSet mimeType="audio/mp4" contentType="audio">
              <Representation id="0" codecs="flac" bandwidth="5400000" audioSamplingRate="192000">
                <SegmentTemplate timescale="192000" initialization="https://sp-pr-cf.audio.tidal.com/init.mp4?hdnea=1&amp;b=2" media="https://sp-pr-cf.audio.tidal.com/$Number$.m4s?hdnea=1&amp;b=2" startNumber="1">
                  <SegmentTimeline>
                    <S d="768000" r="2"/>
                    <S d="384000"/>
                  </SegmentTimeline>
                </SegmentTemplate>
              </Representation>
            </AdaptationSet>
          </Period>
        </MPD>"#;
        let encoded = STANDARD.encode(mpd.as_bytes());
        let stream = parse_manifest("application/dash+xml", &encoded).unwrap();
        assert_eq!(
            stream,
            TidalStream::Segments {
                urls: vec![
                    "https://sp-pr-cf.audio.tidal.com/init.mp4?hdnea=1&b=2".into(),
                    "https://sp-pr-cf.audio.tidal.com/1.m4s?hdnea=1&b=2".into(),
                    "https://sp-pr-cf.audio.tidal.com/2.m4s?hdnea=1&b=2".into(),
                    "https://sp-pr-cf.audio.tidal.com/3.m4s?hdnea=1&b=2".into(),
                    "https://sp-pr-cf.audio.tidal.com/4.m4s?hdnea=1&b=2".into(),
                ],
                ext: "mp4",
            }
        );
    }

    #[test]
    fn parses_tidal_track_and_hires_metadata() {
        let json: Value = serde_json::from_str(
            r#"{
                "id": 77610756,
                "title": "Money",
                "version": "2023 Remaster",
                "duration": 382,
                "trackNumber": 6,
                "volumeNumber": 1,
                "audioQuality": "HI_RES_LOSSLESS",
                "mediaMetadata": { "tags": ["HIRES_LOSSLESS", "LOSSLESS"] },
                "artists": [{ "name": "Pink Floyd" }],
                "album": {
                    "id": 77610750,
                    "title": "The Dark Side of the Moon",
                    "cover": "11111111-2222-3333-4444-555555555555",
                    "releaseDate": "1973-03-01"
                }
            }"#,
        )
        .unwrap();

        let song = parse_track(&json, None).unwrap();
        assert_eq!(song.id, "77610756");
        assert_eq!(song.title, "Money (2023 Remaster)");
        assert_eq!(song.artist.as_deref(), Some("Pink Floyd"));
        assert_eq!(song.album.as_deref(), Some("The Dark Side of the Moon"));
        assert_eq!(song.year, Some(1973));
        assert_eq!(song.bit_depth, Some(24));
        assert_eq!(song.suffix.as_deref(), Some("flac"));
    }
}
