//! Axum HTTP router for the Mobile Remote Web App / PWA.

use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::extract::{Path, Query, Request, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use tauri::Emitter;
use tower::ServiceExt;
use tower_http::services::ServeFile;

use crate::core::mobile_remote::state::{MobileRemoteSharedState, RemoteCmdPayload};
use crate::repository::library::library_files_repository::LibraryFilesRepository;
use crate::repository::library::library_track_repository::LibraryTrackRepository;
use crate::repository::playlist::playlist_item_repository::PlaylistItemRepository;
use crate::repository::playlist::playlist_repository::PlaylistRepository;

const UI_HTML: &str = include_str!("ui.html");

const MANIFEST_JSON: &str = r##"{
  "name": "RustMusic Remote",
  "short_name": "RustMusic",
  "description": "Bit-Perfect Desktop Audio Remote Control",
  "start_url": "/",
  "display": "standalone",
  "background_color": "#090d0b",
  "theme_color": "#090d0b",
  "orientation": "portrait"
}"##;

#[derive(Clone)]
pub struct RouterState {
    pub pool: SqlitePool,
    pub shared: Arc<MobileRemoteSharedState>,
    pub app_handle: tauri::AppHandle,
}

#[derive(Debug, Deserialize, Default)]
pub struct AuthQuery {
    pub pin: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
pub struct PollQuery {
    pub since: Option<u64>,
    pub pin: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
pub struct BrowseQuery {
    pub q: Option<String>,
    pub offset: Option<i64>,
    pub limit: Option<i64>,
    pub sort: Option<String>,
    pub pin: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
pub struct CoverQuery {
    pub path: Option<String>,
    pub pin: Option<String>,
}

#[derive(Debug, Serialize, sqlx::FromRow, Clone)]
pub struct RemoteTrackRow {
    pub id: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub album_id: Option<String>,
    pub artist_id: Option<String>,
    pub path: String,
    pub duration: Option<f64>,
    pub track_number: Option<i32>,
    pub disc_number: Option<i32>,
    pub year: Option<String>,
    pub genre: Option<String>,
    pub audio_format: Option<String>,
    pub bits_per_sample: Option<i32>,
    pub sample_rate: Option<i32>,
    pub thumbnail_path: Option<String>,
    pub favorite: bool,
}

#[derive(Debug, Serialize, sqlx::FromRow, Clone)]
pub struct RemoteAlbumRow {
    pub id: String,
    pub title: String,
    pub artist: String,
    pub year: Option<i32>,
    pub genre: Option<String>,
    pub total_tracks: i64,
    pub thumbnail_path: Option<String>,
}

#[derive(Debug, Serialize, sqlx::FromRow, Clone)]
pub struct RemoteArtistRow {
    pub id: String,
    pub name: String,
    pub total_albums: i64,
    pub total_tracks: i64,
    pub thumbnail_path: Option<String>,
}

#[derive(Debug, Serialize, Clone)]
pub struct RemotePlaylistRow {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub color: String,
    pub icon: String,
    pub track_count: i64,
    pub is_smart: bool,
    pub is_builtin: bool,
    pub cover_path: Option<String>,
}

pub fn build_router(state: RouterState) -> Router {
    Router::new()
        .route("/", get(serve_ui))
        .route("/manifest.json", get(serve_manifest))
        .route("/api/auth", get(check_auth))
        .route("/api/state", get(get_state))
        .route("/api/poll", get(poll_state))
        .route("/api/cmd", post(post_cmd))
        .route("/api/tracks", get(list_tracks))
        .route("/api/albums", get(list_albums))
        .route("/api/albums/{id}", get(get_album_detail))
        .route("/api/artists", get(list_artists))
        .route("/api/artists/{id}", get(get_artist_detail))
        .route("/api/playlists", get(list_playlists))
        .route("/api/playlists/{id}", get(get_playlist_detail))
        .route("/api/cover", get(serve_cover))
        .with_state(state)
}

fn extract_pin<'a>(headers: &'a HeaderMap, query_pin: Option<&'a str>) -> Option<&'a str> {
    headers
        .get("x-remote-pin")
        .and_then(|v| v.to_str().ok())
        .or(query_pin)
}

async fn authorize(
    state: &RouterState,
    headers: &HeaderMap,
    query_pin: Option<&str>,
) -> Result<(), Response> {
    if let Some(cid) = headers.get("x-client-id").and_then(|v| v.to_str().ok()) {
        state.shared.touch_client(cid).await;
    }
    let pin = extract_pin(headers, query_pin);
    if !state.shared.check_pin(pin).await {
        return Err((
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({ "error": "PIN required or invalid", "pin_required": true })),
        )
            .into_response());
    }
    Ok(())
}

async fn serve_ui() -> Response {
    (
        [
            (header::CONTENT_TYPE, "text/html; charset=utf-8"),
            (header::CACHE_CONTROL, "no-cache"),
        ],
        UI_HTML,
    )
        .into_response()
}

async fn serve_manifest() -> Response {
    (
        [(header::CONTENT_TYPE, "application/manifest+json")],
        MANIFEST_JSON,
    )
        .into_response()
}

async fn check_auth(
    State(state): State<RouterState>,
    headers: HeaderMap,
    Query(q): Query<AuthQuery>,
) -> Response {
    if let Some(cid) = headers.get("x-client-id").and_then(|v| v.to_str().ok()) {
        state.shared.touch_client(cid).await;
    }
    let pin_required = state.shared.pin_enabled.load(Ordering::Relaxed);
    let pin = extract_pin(&headers, q.pin.as_deref());
    let authorized = state.shared.check_pin(pin).await;
    Json(serde_json::json!({
        "pin_required": pin_required,
        "authorized": authorized
    }))
    .into_response()
}

async fn get_state(
    State(state): State<RouterState>,
    headers: HeaderMap,
    Query(q): Query<AuthQuery>,
) -> Response {
    if let Err(resp) = authorize(&state, &headers, q.pin.as_deref()).await {
        return resp;
    }
    let snap = state.shared.snapshot().await;
    Json(snap).into_response()
}

async fn poll_state(
    State(state): State<RouterState>,
    headers: HeaderMap,
    Query(q): Query<PollQuery>,
) -> Response {
    if let Err(resp) = authorize(&state, &headers, q.pin.as_deref()).await {
        return resp;
    }

    let since = q.since.unwrap_or(0);
    {
        let current = state.shared.player.read().await;
        if since == 0 || (current.updated_at_ms != 0 && current.updated_at_ms != since) {
            return Json(current.clone()).into_response();
        }
    }

    let mut rx = state.shared.state_tx.subscribe();
    let _ = tokio::time::timeout(Duration::from_secs(15), rx.recv()).await;
    let snap = state.shared.snapshot().await;
    Json(snap).into_response()
}

async fn post_cmd(
    State(state): State<RouterState>,
    headers: HeaderMap,
    Query(q): Query<AuthQuery>,
    Json(mut payload): Json<RemoteCmdPayload>,
) -> Response {
    if let Err(resp) = authorize(&state, &headers, q.pin.as_deref()).await {
        return resp;
    }

    if payload.cmd == "add-to-playlist" {
        let Some(pl_str) = payload.playlist_id.as_deref() else {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": "Missing playlist_id" })),
            )
                .into_response();
        };
        let Ok(playlist_id) = pl_str.parse::<i64>() else {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": "Invalid playlist_id" })),
            )
                .into_response();
        };

        let resolved_track_id = if let Some(tid) = payload.track_id.clone() {
            Some(tid)
        } else if let Some(ref path) = payload.path {
            if let Ok(Some(file)) = LibraryFilesRepository::find_by_path_any(&state.pool, path).await
            {
                LibraryTrackRepository::find_by_file_id(&state.pool, &file.id)
                    .await
                    .ok()
                    .flatten()
                    .map(|t| t.id)
            } else {
                None
            }
        } else {
            None
        };

        let Some(track_id) = resolved_track_id else {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({ "error": "Track not found in library" })),
            )
                .into_response();
        };

        let sort_index = PlaylistItemRepository::get_next_sort_index(&state.pool, playlist_id)
            .await
            .unwrap_or(0);
        if let Err(e) =
            PlaylistItemRepository::insert_playlist_item(&state.pool, playlist_id, track_id, sort_index)
                .await
        {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
                .into_response();
        }
        let _ = PlaylistRepository::recalculate_stats(&state.pool, playlist_id).await;
        payload.cmd = "refresh-playlists".to_string();
    }

    let _ = state.app_handle.emit("mobile-remote-cmd", &payload);
    Json(serde_json::json!({ "ok": true })).into_response()
}

pub async fn query_tracks(
    pool: &SqlitePool,
    search: &str,
    sort: Option<&str>,
    limit: i64,
    offset: i64,
) -> (Vec<RemoteTrackRow>, i64) {
    let search_lower = search.trim().to_lowercase();
    let pattern = format!("%{search_lower}%");
    let has_search = !search_lower.is_empty();

    let order_clause = match sort {
        Some("recent") => "lt.created_at DESC, lt.title COLLATE NOCASE ASC",
        Some("artist") => "COALESCE(a.name, lc.artist, '') COLLATE NOCASE ASC, COALESCE(la.title, lc.album, '') COLLATE NOCASE ASC, COALESCE(lt.track_number, lc.track_number, 0) ASC",
        Some("album") => "COALESCE(la.title, lc.album, '') COLLATE NOCASE ASC, COALESCE(lt.disc_number, lc.disc_number, 1) ASC, COALESCE(lt.track_number, lc.track_number, 0) ASC",
        _ => "lt.title COLLATE NOCASE ASC",
    };

    let total: i64 = sqlx::query_scalar(
        r#"
        SELECT COUNT(*)
        FROM library_tracks lt
        INNER JOIN library_files lf ON lf.id = lt.file_id
            AND NOT EXISTS (SELECT 1 FROM library_dirs ld WHERE ld.id = lf.library_dir_id AND ld.is_active = 0)
        LEFT JOIN library_cache lc ON lc.id = lt.cache_id
        LEFT JOIN library_albums la ON la.id = lt.library_album_id
        LEFT JOIN artists a ON a.id = lt.artist_id
        WHERE (?1 = 0 OR LOWER(COALESCE(lt.title, '')) LIKE ?2
               OR LOWER(COALESCE(a.name, lc.artist, '')) LIKE ?2
               OR LOWER(COALESCE(la.title, lc.album, '')) LIKE ?2)
        "#,
    )
    .bind(has_search as i64)
    .bind(&pattern)
    .fetch_one(pool)
    .await
    .unwrap_or(0);

    let sql = format!(
        r#"
        SELECT
            lt.id AS id,
            COALESCE(NULLIF(lt.title, ''), lf.filename) AS title,
            COALESCE(NULLIF(a.name, ''), NULLIF(lc.artist, ''), 'Unknown Artist') AS artist,
            COALESCE(NULLIF(la.title, ''), NULLIF(lc.album, ''), '') AS album,
            lt.library_album_id AS album_id,
            lt.artist_id AS artist_id,
            lf.path AS path,
            COALESCE(lt.duration, lc.duration) AS duration,
            COALESCE(lt.track_number, lc.track_number) AS track_number,
            COALESCE(lt.disc_number, lc.disc_number, 1) AS disc_number,
            COALESCE(CAST(la.year AS TEXT), lc.year) AS year,
            COALESCE(lc.genre, la.genre) AS genre,
            lc.audio_format AS audio_format,
            lc.bits_per_sample AS bits_per_sample,
            COALESCE(lt.sample_rate, lc.sample_rate) AS sample_rate,
            COALESCE(NULLIF(lc.thumbnail_path, ''), NULLIF(la.cover_url, '')) AS thumbnail_path,
            COALESCE(lt.favorite, 0) AS favorite
        FROM library_tracks lt
        INNER JOIN library_files lf ON lf.id = lt.file_id
            AND NOT EXISTS (SELECT 1 FROM library_dirs ld WHERE ld.id = lf.library_dir_id AND ld.is_active = 0)
        LEFT JOIN library_cache lc ON lc.id = lt.cache_id
        LEFT JOIN library_albums la ON la.id = lt.library_album_id
        LEFT JOIN artists a ON a.id = lt.artist_id
        WHERE (?1 = 0 OR LOWER(COALESCE(lt.title, '')) LIKE ?2
               OR LOWER(COALESCE(a.name, lc.artist, '')) LIKE ?2
               OR LOWER(COALESCE(la.title, lc.album, '')) LIKE ?2)
        ORDER BY {order_clause}
        LIMIT ?3 OFFSET ?4
        "#
    );

    let tracks = match sqlx::query_as::<_, RemoteTrackRow>(sqlx::AssertSqlSafe(sql))
        .bind(has_search as i64)
        .bind(&pattern)
        .bind(limit)
        .bind(offset)
        .fetch_all(pool)
        .await
    {
        Ok(rows) => rows,
        Err(e) => {
            log::error!("[mobile-remote] query_tracks SQL error: {e}");
            Vec::new()
        }
    };

    (tracks, total)
}

async fn list_tracks(
    State(state): State<RouterState>,
    headers: HeaderMap,
    Query(q): Query<BrowseQuery>,
) -> Response {
    if let Err(resp) = authorize(&state, &headers, q.pin.as_deref()).await {
        return resp;
    }

    let search = q.q.unwrap_or_default();
    let limit = q.limit.unwrap_or(60).clamp(1, 200);
    let offset = q.offset.unwrap_or(0).max(0);
    let (tracks, total) = query_tracks(&state.pool, &search, q.sort.as_deref(), limit, offset).await;

    Json(serde_json::json!({
        "tracks": tracks,
        "total": total,
        "offset": offset,
        "limit": limit
    }))
    .into_response()
}

pub async fn query_albums(
    pool: &SqlitePool,
    search: &str,
    limit: i64,
    offset: i64,
) -> Vec<RemoteAlbumRow> {
    let search_lower = search.trim().to_lowercase();
    let pattern = format!("%{search_lower}%");
    let has_search = !search_lower.is_empty();

    match sqlx::query_as::<_, RemoteAlbumRow>(
        r#"
        SELECT
            la.id AS id,
            la.title AS title,
            COALESCE(NULLIF(a.name, ''), 'Unknown Artist') AS artist,
            la.year AS year,
            la.genre AS genre,
            COUNT(lt.id) AS total_tracks,
            COALESCE(NULLIF(la.cover_url, ''), MAX(lc.thumbnail_path)) AS thumbnail_path
        FROM library_albums la
        INNER JOIN library_tracks lt ON lt.library_album_id = la.id
        INNER JOIN library_files lf ON lf.id = lt.file_id
            AND NOT EXISTS (SELECT 1 FROM library_dirs ld WHERE ld.id = lf.library_dir_id AND ld.is_active = 0)
        LEFT JOIN library_cache lc ON lc.id = lt.cache_id
        LEFT JOIN artists a ON a.id = la.artist_id
        WHERE (?1 = 0 OR LOWER(la.title) LIKE ?2 OR LOWER(COALESCE(a.name, '')) LIKE ?2)
        GROUP BY la.id
        HAVING COUNT(lt.id) > 0
        ORDER BY la.title COLLATE NOCASE ASC
        LIMIT ?3 OFFSET ?4
        "#,
    )
    .bind(has_search as i64)
    .bind(&pattern)
    .bind(limit)
    .bind(offset)
    .fetch_all(pool)
    .await
    {
        Ok(rows) => rows,
        Err(e) => {
            log::error!("[mobile-remote] query_albums SQL error: {e}");
            Vec::new()
        }
    }
}

async fn list_albums(
    State(state): State<RouterState>,
    headers: HeaderMap,
    Query(q): Query<BrowseQuery>,
) -> Response {
    if let Err(resp) = authorize(&state, &headers, q.pin.as_deref()).await {
        return resp;
    }

    let search = q.q.unwrap_or_default();
    let limit = q.limit.unwrap_or(60).clamp(1, 200);
    let offset = q.offset.unwrap_or(0).max(0);
    let albums = query_albums(&state.pool, &search, limit, offset).await;

    Json(serde_json::json!({
        "albums": albums,
        "offset": offset,
        "limit": limit
    }))
    .into_response()
}

async fn get_album_detail(
    Path(album_id): Path<String>,
    State(state): State<RouterState>,
    headers: HeaderMap,
    Query(q): Query<AuthQuery>,
) -> Response {
    if let Err(resp) = authorize(&state, &headers, q.pin.as_deref()).await {
        return resp;
    }

    let album: Option<RemoteAlbumRow> = sqlx::query_as::<_, RemoteAlbumRow>(
        r#"
        SELECT
            la.id AS id,
            la.title AS title,
            COALESCE(NULLIF(a.name, ''), 'Unknown Artist') AS artist,
            la.year AS year,
            la.genre AS genre,
            COUNT(lt.id) AS total_tracks,
            COALESCE(NULLIF(la.cover_url, ''), MAX(lc.thumbnail_path)) AS thumbnail_path
        FROM library_albums la
        LEFT JOIN library_tracks lt ON lt.library_album_id = la.id
        LEFT JOIN library_files lf ON lf.id = lt.file_id
        LEFT JOIN library_cache lc ON lc.id = lt.cache_id
        LEFT JOIN artists a ON a.id = la.artist_id
        WHERE la.id = ?1
        GROUP BY la.id
        "#,
    )
    .bind(&album_id)
    .fetch_optional(&state.pool)
    .await
    .ok()
    .flatten();

    let tracks: Vec<RemoteTrackRow> = sqlx::query_as::<_, RemoteTrackRow>(
        r#"
        SELECT
            lt.id AS id,
            COALESCE(NULLIF(lt.title, ''), lf.filename) AS title,
            COALESCE(NULLIF(a.name, ''), NULLIF(lc.artist, ''), 'Unknown Artist') AS artist,
            COALESCE(NULLIF(la.title, ''), NULLIF(lc.album, ''), '') AS album,
            lt.library_album_id AS album_id,
            lt.artist_id AS artist_id,
            lf.path AS path,
            COALESCE(lt.duration, lc.duration) AS duration,
            COALESCE(lt.track_number, lc.track_number) AS track_number,
            COALESCE(lt.disc_number, lc.disc_number, 1) AS disc_number,
            COALESCE(CAST(la.year AS TEXT), lc.year) AS year,
            COALESCE(lc.genre, la.genre) AS genre,
            lc.audio_format AS audio_format,
            lc.bits_per_sample AS bits_per_sample,
            COALESCE(lt.sample_rate, lc.sample_rate) AS sample_rate,
            COALESCE(NULLIF(lc.thumbnail_path, ''), NULLIF(la.cover_url, '')) AS thumbnail_path,
            COALESCE(lt.favorite, 0) AS favorite
        FROM library_tracks lt
        INNER JOIN library_files lf ON lf.id = lt.file_id
            AND NOT EXISTS (SELECT 1 FROM library_dirs ld WHERE ld.id = lf.library_dir_id AND ld.is_active = 0)
        LEFT JOIN library_cache lc ON lc.id = lt.cache_id
        LEFT JOIN library_albums la ON la.id = lt.library_album_id
        LEFT JOIN artists a ON a.id = lt.artist_id
        WHERE lt.library_album_id = ?1
        ORDER BY COALESCE(lt.disc_number, lc.disc_number, 1) ASC,
                 COALESCE(lt.track_number, lc.track_number, 0) ASC,
                 lt.title COLLATE NOCASE ASC
        "#,
    )
    .bind(&album_id)
    .fetch_all(&state.pool)
    .await
    .unwrap_or_default();

    Json(serde_json::json!({
        "album": album,
        "tracks": tracks
    }))
    .into_response()
}

pub async fn query_artists(
    pool: &SqlitePool,
    search: &str,
    limit: i64,
    offset: i64,
) -> Vec<RemoteArtistRow> {
    let search_lower = search.trim().to_lowercase();
    let pattern = format!("%{search_lower}%");
    let has_search = !search_lower.is_empty();

    match sqlx::query_as::<_, RemoteArtistRow>(
        r#"
        SELECT
            a.id AS id,
            a.name AS name,
            COUNT(DISTINCT lt.library_album_id) AS total_albums,
            COUNT(lt.id) AS total_tracks,
            COALESCE(NULLIF(a.image_url, ''), MAX(lc.thumbnail_path)) AS thumbnail_path
        FROM artists a
        INNER JOIN library_tracks lt ON lt.artist_id = a.id
        INNER JOIN library_files lf ON lf.id = lt.file_id
            AND NOT EXISTS (SELECT 1 FROM library_dirs ld WHERE ld.id = lf.library_dir_id AND ld.is_active = 0)
        LEFT JOIN library_cache lc ON lc.id = lt.cache_id
        WHERE (?1 = 0 OR LOWER(a.name) LIKE ?2)
        GROUP BY a.id
        HAVING COUNT(lt.id) > 0
        ORDER BY a.name COLLATE NOCASE ASC
        LIMIT ?3 OFFSET ?4
        "#,
    )
    .bind(has_search as i64)
    .bind(&pattern)
    .bind(limit)
    .bind(offset)
    .fetch_all(pool)
    .await
    {
        Ok(rows) => rows,
        Err(e) => {
            log::error!("[mobile-remote] query_artists SQL error: {e}");
            Vec::new()
        }
    }
}

async fn list_artists(
    State(state): State<RouterState>,
    headers: HeaderMap,
    Query(q): Query<BrowseQuery>,
) -> Response {
    if let Err(resp) = authorize(&state, &headers, q.pin.as_deref()).await {
        return resp;
    }

    let search = q.q.unwrap_or_default();
    let limit = q.limit.unwrap_or(60).clamp(1, 200);
    let offset = q.offset.unwrap_or(0).max(0);
    let artists = query_artists(&state.pool, &search, limit, offset).await;

    Json(serde_json::json!({
        "artists": artists,
        "offset": offset,
        "limit": limit
    }))
    .into_response()
}

async fn get_artist_detail(
    Path(artist_id): Path<String>,
    State(state): State<RouterState>,
    headers: HeaderMap,
    Query(q): Query<AuthQuery>,
) -> Response {
    if let Err(resp) = authorize(&state, &headers, q.pin.as_deref()).await {
        return resp;
    }

    let artist_name: String = sqlx::query_scalar("SELECT name FROM artists WHERE id = ?1")
        .bind(&artist_id)
        .fetch_optional(&state.pool)
        .await
        .ok()
        .flatten()
        .unwrap_or_else(|| "Artist".to_string());

    let albums: Vec<RemoteAlbumRow> = sqlx::query_as::<_, RemoteAlbumRow>(
        r#"
        SELECT
            la.id AS id,
            la.title AS title,
            COALESCE(NULLIF(a.name, ''), 'Unknown Artist') AS artist,
            la.year AS year,
            la.genre AS genre,
            COUNT(lt.id) AS total_tracks,
            COALESCE(NULLIF(la.cover_url, ''), MAX(lc.thumbnail_path)) AS thumbnail_path
        FROM library_albums la
        INNER JOIN library_tracks lt ON lt.library_album_id = la.id
        INNER JOIN library_files lf ON lf.id = lt.file_id
            AND NOT EXISTS (SELECT 1 FROM library_dirs ld WHERE ld.id = lf.library_dir_id AND ld.is_active = 0)
        LEFT JOIN library_cache lc ON lc.id = lt.cache_id
        LEFT JOIN artists a ON a.id = la.artist_id
        WHERE la.artist_id = ?1 OR lt.artist_id = ?1
        GROUP BY la.id
        ORDER BY la.year DESC, la.title COLLATE NOCASE ASC
        "#,
    )
    .bind(&artist_id)
    .fetch_all(&state.pool)
    .await
    .unwrap_or_default();

    let tracks: Vec<RemoteTrackRow> = sqlx::query_as::<_, RemoteTrackRow>(
        r#"
        SELECT
            lt.id AS id,
            COALESCE(NULLIF(lt.title, ''), lf.filename) AS title,
            COALESCE(NULLIF(a.name, ''), NULLIF(lc.artist, ''), 'Unknown Artist') AS artist,
            COALESCE(NULLIF(la.title, ''), NULLIF(lc.album, ''), '') AS album,
            lt.library_album_id AS album_id,
            lt.artist_id AS artist_id,
            lf.path AS path,
            COALESCE(lt.duration, lc.duration) AS duration,
            COALESCE(lt.track_number, lc.track_number) AS track_number,
            COALESCE(lt.disc_number, lc.disc_number, 1) AS disc_number,
            COALESCE(CAST(la.year AS TEXT), lc.year) AS year,
            COALESCE(lc.genre, la.genre) AS genre,
            lc.audio_format AS audio_format,
            lc.bits_per_sample AS bits_per_sample,
            COALESCE(lt.sample_rate, lc.sample_rate) AS sample_rate,
            COALESCE(NULLIF(lc.thumbnail_path, ''), NULLIF(la.cover_url, '')) AS thumbnail_path,
            COALESCE(lt.favorite, 0) AS favorite
        FROM library_tracks lt
        INNER JOIN library_files lf ON lf.id = lt.file_id
            AND NOT EXISTS (SELECT 1 FROM library_dirs ld WHERE ld.id = lf.library_dir_id AND ld.is_active = 0)
        LEFT JOIN library_cache lc ON lc.id = lt.cache_id
        LEFT JOIN library_albums la ON la.id = lt.library_album_id
        LEFT JOIN artists a ON a.id = lt.artist_id
        WHERE lt.artist_id = ?1
        ORDER BY COALESCE(la.year, 0) DESC,
                 COALESCE(la.title, '') COLLATE NOCASE ASC,
                 COALESCE(lt.disc_number, lc.disc_number, 1) ASC,
                 COALESCE(lt.track_number, lc.track_number, 0) ASC
        LIMIT 200
        "#,
    )
    .bind(&artist_id)
    .fetch_all(&state.pool)
    .await
    .unwrap_or_default();

    Json(serde_json::json!({
        "artist": { "id": artist_id, "name": artist_name },
        "albums": albums,
        "tracks": tracks
    }))
    .into_response()
}

pub async fn query_playlists(pool: &SqlitePool) -> Vec<RemotePlaylistRow> {
    let liked_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM track_liked")
        .fetch_one(pool)
        .await
        .unwrap_or(0);

    let recent_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM recent_files")
        .fetch_one(pool)
        .await
        .unwrap_or(0);

    let mut result = vec![
        RemotePlaylistRow {
            id: "liked".to_string(),
            name: "Liked Tracks".to_string(),
            description: Some("Your favorite tracks".to_string()),
            color: "#f43f5e".to_string(),
            icon: "heart".to_string(),
            track_count: liked_count,
            is_smart: false,
            is_builtin: true,
            cover_path: None,
        },
        RemotePlaylistRow {
            id: "recent".to_string(),
            name: "Recently Played".to_string(),
            description: Some("Listening history".to_string()),
            color: "#0ea5e9".to_string(),
            icon: "history".to_string(),
            track_count: recent_count,
            is_smart: false,
            is_builtin: true,
            cover_path: None,
        },
    ];

    let profile_ids: Vec<i64> = sqlx::query_scalar("SELECT id FROM profil ORDER BY id ASC")
        .fetch_all(pool)
        .await
        .unwrap_or_else(|_| vec![1]);

    for pid in profile_ids {
        if let Ok(playlists) = PlaylistRepository::find_all_by_profile_id(pool, pid, None).await {
            for p in playlists.into_iter().filter(|p| !p.is_mix) {
                let mut count = p.track_count;
                if p.is_smart {
                    if let Some(json) = PlaylistRepository::find_rules(pool, p.id)
                        .await
                        .ok()
                        .flatten()
                    {
                        if let Ok(rules) = serde_json::from_str(&json) {
                            if let Ok(n) = crate::core::smart_playlist::engine::count(
                                pool,
                                &rules,
                                Some(p.source_id),
                            )
                            .await
                            {
                                count = n;
                            }
                        }
                    }
                }
                result.push(RemotePlaylistRow {
                    id: p.id.to_string(),
                    name: p.name,
                    description: p.description,
                    color: p.color,
                    icon: p.icon,
                    track_count: count,
                    is_smart: p.is_smart || p.is_auto,
                    is_builtin: false,
                    cover_path: p.cover,
                });
            }
        }
    }

    result
}

async fn list_playlists(
    State(state): State<RouterState>,
    headers: HeaderMap,
    Query(q): Query<AuthQuery>,
) -> Response {
    if let Err(resp) = authorize(&state, &headers, q.pin.as_deref()).await {
        return resp;
    }

    let result = query_playlists(&state.pool).await;
    Json(serde_json::json!({ "playlists": result })).into_response()
}

async fn get_playlist_detail(
    Path(playlist_id): Path<String>,
    State(state): State<RouterState>,
    headers: HeaderMap,
    Query(q): Query<AuthQuery>,
) -> Response {
    if let Err(resp) = authorize(&state, &headers, q.pin.as_deref()).await {
        return resp;
    }

    if playlist_id == "liked" {
        let tracks: Vec<RemoteTrackRow> = sqlx::query_as::<_, RemoteTrackRow>(
            r#"
            SELECT
                COALESCE(lt.id, tl.path) AS id,
                COALESCE(NULLIF(lt.title, ''), NULLIF(lc.title, ''), tl.path) AS title,
                COALESCE(NULLIF(a.name, ''), NULLIF(lc.artist, ''), 'Unknown Artist') AS artist,
                COALESCE(NULLIF(la.title, ''), NULLIF(lc.album, ''), '') AS album,
                lt.library_album_id AS album_id,
                lt.artist_id AS artist_id,
                tl.path AS path,
                COALESCE(lt.duration, lc.duration) AS duration,
                COALESCE(lt.track_number, lc.track_number) AS track_number,
                COALESCE(lt.disc_number, lc.disc_number, 1) AS disc_number,
                COALESCE(CAST(la.year AS TEXT), lc.year) AS year,
                COALESCE(lc.genre, la.genre) AS genre,
                lc.audio_format AS audio_format,
                lc.bits_per_sample AS bits_per_sample,
                COALESCE(lt.sample_rate, lc.sample_rate) AS sample_rate,
                COALESCE(NULLIF(lc.thumbnail_path, ''), NULLIF(la.cover_url, '')) AS thumbnail_path,
                1 AS favorite
            FROM track_liked tl
            LEFT JOIN library_files lf ON lf.path = tl.path
            LEFT JOIN library_tracks lt ON lt.file_id = lf.id
            LEFT JOIN library_cache lc ON lc.id = COALESCE(lt.cache_id, tl.library_cache_id)
            LEFT JOIN library_albums la ON la.id = lt.library_album_id
            LEFT JOIN artists a ON a.id = lt.artist_id
            ORDER BY tl.created_at DESC
            LIMIT 300
            "#,
        )
        .fetch_all(&state.pool)
        .await
        .unwrap_or_else(|e| {
            log::error!("[mobile-remote] liked tracks SQL error: {e}");
            Vec::new()
        });

        return Json(serde_json::json!({
            "playlist": {
                "id": "liked",
                "name": "Liked Tracks",
                "color": "#f43f5e",
                "is_builtin": true,
                "track_count": tracks.len()
            },
            "tracks": tracks
        }))
        .into_response();
    }

    if playlist_id == "recent" {
        let tracks: Vec<RemoteTrackRow> = sqlx::query_as::<_, RemoteTrackRow>(
            r#"
            SELECT
                COALESCE(lt.id, rf.path) AS id,
                COALESCE(NULLIF(lt.title, ''), NULLIF(lc.title, ''), rf.path) AS title,
                COALESCE(NULLIF(a.name, ''), NULLIF(lc.artist, ''), 'Unknown Artist') AS artist,
                COALESCE(NULLIF(la.title, ''), NULLIF(lc.album, ''), '') AS album,
                lt.library_album_id AS album_id,
                lt.artist_id AS artist_id,
                rf.path AS path,
                COALESCE(lt.duration, lc.duration) AS duration,
                COALESCE(lt.track_number, lc.track_number) AS track_number,
                COALESCE(lt.disc_number, lc.disc_number, 1) AS disc_number,
                COALESCE(CAST(la.year AS TEXT), lc.year) AS year,
                COALESCE(lc.genre, la.genre) AS genre,
                lc.audio_format AS audio_format,
                lc.bits_per_sample AS bits_per_sample,
                COALESCE(lt.sample_rate, lc.sample_rate) AS sample_rate,
                COALESCE(NULLIF(lc.thumbnail_path, ''), NULLIF(la.cover_url, '')) AS thumbnail_path,
                COALESCE(lt.favorite, 0) AS favorite
            FROM recent_files rf
            LEFT JOIN library_files lf ON lf.path = rf.path
            LEFT JOIN library_tracks lt ON lt.file_id = lf.id
            LEFT JOIN library_cache lc ON lc.id = COALESCE(lt.cache_id, rf.library_cache_id)
            LEFT JOIN library_albums la ON la.id = lt.library_album_id
            LEFT JOIN artists a ON a.id = lt.artist_id
            ORDER BY rf.last_played_at DESC
            LIMIT 200
            "#,
        )
        .fetch_all(&state.pool)
        .await
        .unwrap_or_else(|e| {
            log::error!("[mobile-remote] recent tracks SQL error: {e}");
            Vec::new()
        });

        return Json(serde_json::json!({
            "playlist": {
                "id": "recent",
                "name": "Recently Played",
                "color": "#0ea5e9",
                "is_builtin": true,
                "track_count": tracks.len()
            },
            "tracks": tracks
        }))
        .into_response();
    }

    let Ok(pl_id) = playlist_id.parse::<i64>() else {
        return (StatusCode::BAD_REQUEST, "Invalid playlist ID").into_response();
    };

    let Some(pl) = PlaylistRepository::find_by_id(&state.pool, pl_id)
        .await
        .ok()
        .flatten()
    else {
        return (StatusCode::NOT_FOUND, "Playlist not found").into_response();
    };

    if pl.is_smart {
        if let Some(json) = PlaylistRepository::find_rules(&state.pool, pl.id)
            .await
            .ok()
            .flatten()
        {
            if let Ok(rules) = serde_json::from_str::<crate::core::smart_playlist::rules::SmartRules>(&json) {
                if let Ok(items) = crate::core::smart_playlist::engine::evaluate(
                    &state.pool,
                    pl.id,
                    &rules,
                    Some(pl.source_id),
                )
                .await
                {
                    let tracks: Vec<RemoteTrackRow> = items
                        .into_iter()
                        .filter_map(|t| {
                            let path = t.path?;
                            Some(RemoteTrackRow {
                                id: t.library_track_id,
                                title: t.title.unwrap_or_else(|| "Untitled".to_string()),
                                artist: t.artist_name.unwrap_or_else(|| "Unknown Artist".to_string()),
                                album: t.album_title.unwrap_or_default(),
                                album_id: t.album_id,
                                artist_id: t.artist_id,
                                path,
                                duration: t.duration,
                                track_number: t.track_number.map(|n| n as i32),
                                disc_number: t.disc_number.map(|n| n as i32),
                                year: None,
                                genre: None,
                                audio_format: None,
                                bits_per_sample: None,
                                sample_rate: None,
                                thumbnail_path: t.thumbnail_path,
                                favorite: false,
                            })
                        })
                        .collect();

                    return Json(serde_json::json!({
                        "playlist": {
                            "id": pl.id.to_string(),
                            "name": pl.name,
                            "description": pl.description,
                            "color": pl.color,
                            "is_smart": true,
                            "track_count": tracks.len()
                        },
                        "tracks": tracks
                    }))
                    .into_response();
                }
            }
        }
    }

    let tracks: Vec<RemoteTrackRow> = sqlx::query_as::<_, RemoteTrackRow>(
        r#"
        SELECT
            lt.id AS id,
            COALESCE(NULLIF(lt.title, ''), lf.filename) AS title,
            COALESCE(NULLIF(a.name, ''), NULLIF(lc.artist, ''), 'Unknown Artist') AS artist,
            COALESCE(NULLIF(la.title, ''), NULLIF(lc.album, ''), '') AS album,
            lt.library_album_id AS album_id,
            lt.artist_id AS artist_id,
            lf.path AS path,
            COALESCE(lt.duration, lc.duration) AS duration,
            COALESCE(lt.track_number, lc.track_number) AS track_number,
            COALESCE(lt.disc_number, lc.disc_number, 1) AS disc_number,
            COALESCE(CAST(la.year AS TEXT), lc.year) AS year,
            COALESCE(lc.genre, la.genre) AS genre,
            lc.audio_format AS audio_format,
            lc.bits_per_sample AS bits_per_sample,
            COALESCE(lt.sample_rate, lc.sample_rate) AS sample_rate,
            COALESCE(NULLIF(lc.thumbnail_path, ''), NULLIF(la.cover_url, '')) AS thumbnail_path,
            COALESCE(lt.favorite, 0) AS favorite
        FROM playlist_items pi
        INNER JOIN library_tracks lt ON lt.id = pi.library_track_id
        INNER JOIN library_files lf ON lf.id = lt.file_id
        LEFT JOIN library_cache lc ON lc.id = lt.cache_id
        LEFT JOIN library_albums la ON la.id = lt.library_album_id
        LEFT JOIN artists a ON a.id = lt.artist_id
        WHERE pi.playlist_id = ?1
        ORDER BY pi.sort_index ASC, pi.id ASC
        "#,
    )
    .bind(pl_id)
    .fetch_all(&state.pool)
    .await
    .unwrap_or_else(|e| {
        log::error!("[mobile-remote] playlist tracks SQL error: {e}");
        Vec::new()
    });

    Json(serde_json::json!({
        "playlist": {
            "id": pl.id.to_string(),
            "name": pl.name,
            "description": pl.description,
            "color": pl.color,
            "is_smart": pl.is_smart || pl.is_auto,
            "track_count": tracks.len()
        },
        "tracks": tracks
    }))
    .into_response()
}

async fn serve_cover(
    State(state): State<RouterState>,
    headers: HeaderMap,
    Query(q): Query<CoverQuery>,
    request: Request<Body>,
) -> Response {
    if let Err(resp) = authorize(&state, &headers, q.pin.as_deref()).await {
        return resp;
    }

    let Some(raw_path) = q.path.as_deref().map(str::trim).filter(|s| !s.is_empty()) else {
        return StatusCode::NOT_FOUND.into_response();
    };

    if raw_path.starts_with("http://") || raw_path.starts_with("https://") {
        return Redirect::temporary(raw_path).into_response();
    }

    if raw_path.starts_with("data:") {
        if let Some((meta, b64)) = raw_path.split_once(',') {
            use base64::Engine;
            if let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(b64) {
                let mime = meta
                    .strip_prefix("data:")
                    .and_then(|s| s.split(';').next())
                    .unwrap_or("image/jpeg");
                return ([(header::CONTENT_TYPE, mime)], bytes).into_response();
            }
        }
        return StatusCode::NOT_FOUND.into_response();
    }

    let path = PathBuf::from(raw_path);
    if !path.is_file() {
        return StatusCode::NOT_FOUND.into_response();
    }

    match ServeFile::new(&path).oneshot(request).await {
        Ok(resp) => {
            let mut resp = resp.map(Body::new).into_response();
            resp.headers_mut().insert(
                header::CACHE_CONTROL,
                HeaderValue::from_static("public, max-age=86400"),
            );
            resp
        }
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn queries_succeed_against_real_sqlite_schema() {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        sqlx::migrate!("./SQL/").run(&pool).await.unwrap();

        let (tracks, total) = query_tracks(&pool, "", Some("title"), 50, 0).await;
        assert_eq!(tracks.len(), 0);
        assert_eq!(total, 0);

        let albums = query_albums(&pool, "test", 50, 0).await;
        assert_eq!(albums.len(), 0);

        let artists = query_artists(&pool, "", 50, 0).await;
        assert_eq!(artists.len(), 0);

        let playlists = query_playlists(&pool).await;
        assert!(playlists.len() >= 2);
        assert_eq!(playlists[0].id, "liked");
        assert_eq!(playlists[1].id, "recent");
    }
}
