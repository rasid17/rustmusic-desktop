use tauri::State;
use crate::repository::library::library_files_repository::LibraryFilesRepository;
use crate::repository::settings::settings_repository::SettingsRepository;

use crate::core::audio_lyrics::{lrclib_client, lyrics_ovh_client, sidecar};
use crate::entity::audio_lyrics::lyrics::{Lyrics, LyricsSource, LyricsUpsert};
use crate::repository::audio_lyrics::lyrics_repository::LyricsRepository;
use crate::repository::library::library_track_repository::LibraryTrackRepository;
use crate::state::AppState;

/// Récupère le `library_files.id` d'un chemin sans décoder l'entité LibraryFile
/// complète (qui a un champ `modified_at` parfois stocké en INTEGER mais déclaré
/// en `Option<String>` dans l'entity → conflit de décodage).
async fn find_file_id_by_path(
    pool: &sqlx::SqlitePool,
    path: &str,
) -> Result<Option<String>, sqlx::Error> {
    LibraryFilesRepository::find_id_by_path(pool, path).await
}

/// Source de paroles en ligne activée dans les réglages ? Active sauf « false »
/// explicite — une clé absente de la base vaut son défaut.
async fn reglage_actif(pool: &sqlx::SqlitePool, cle: &str) -> bool {
    SettingsRepository::get(pool, cle)
        .await
        .ok()
        .flatten()
        .as_deref()
        != Some("false")
}

/// Récupère les paroles d'un morceau via son chemin de fichier audio.
///
/// On accepte le `path` plutôt qu'un track_id parce que le player garde
/// le `path` en référence stable, alors que `trackId` côté front correspond
/// au queueId (pas à `library_tracks.id`).
///
/// Cascade :
/// 1. Lookup library_files via path → file_id
/// 2. Lookup library_tracks via file_id → métadonnées + library_track_id
/// 3. Cache DB (renvoyé tel quel même si partiel ou "none")
/// 4. Sidecar `.lrc` à côté du fichier
/// 5. LRCLIB en ligne (throttlé, avec pause après rejet), si activé en réglages
/// 6. Repli lyrics.ovh (texte simple seul), si activé
/// 7. Rien trouvé → marque `source=none` pour ne pas re-spammer ; une panne
///    de service n'est pas une absence et ne se met pas en cache.
#[tauri::command]
pub async fn get_lyrics(
    state: State<'_, AppState>,
    path: String,
) -> Result<Option<Lyrics>, String> {

    // 1. Lookup file_id via path (requête scalaire)
    let file_id = find_file_id_by_path(&state.pool, &path)
        .await
        .map_err(|e| format!("DB error : {}", e))?
        .ok_or_else(|| format!("Fichier introuvable en base : {}", path))?;

    // 2. Lookup track via file_id
    let track = LibraryTrackRepository::find_track_view_by_file_id(&state.pool, &file_id)
        .await
        .map_err(|e| format!("DB error : {}", e))?
        .ok_or_else(|| format!("Track introuvable pour ce fichier : {}", path))?;

    let track_id = track.id.clone();

    // 3. Cache DB
    if let Ok(Some(cached)) = LyricsRepository::find_by_track_id(&state.pool, &track_id).await {
        return Ok(Some(cached));
    }

    // 4. Sidecar .lrc
    if let Some(lrc) = sidecar::read_sidecar_lrc(&track.path) {
        let synced = if sidecar::has_synced_timestamps(&lrc) {
            Some(lrc.clone())
        } else {
            None
        };
        let plain = if synced.is_none() { Some(lrc) } else { None };

        let saved = LyricsRepository::upsert(
            &state.pool,
            LyricsUpsert {
                track_id: track_id.clone(),
                plain,
                synced,
                source: LyricsSource::Sidecar,
                lrclib_id: None,
            },
        )
        .await
        .map_err(|e| format!("DB upsert error : {}", e))?;

        return Ok(Some(saved));
    }

    // 5. LRCLIB en ligne (paroles synchronisées si possible)
    let artist = track.artist.unwrap_or_default();
    let title = track.title.clone();
    let album = track.album;
    let duration = track.duration.unwrap_or(0.0).round() as u32;

    if artist.is_empty() || title.is_empty() {
        let _ = LyricsRepository::mark_not_found(&state.pool, &track_id).await;
        return Ok(None);
    }

    let lrclib = if reglage_actif(&state.pool, "lyrics_lrclib").await {
        lrclib_client::fetch_lyrics(&artist, &title, album.as_deref(), duration).await
    } else {
        Ok(None)
    };

    if let Ok(Some(resp)) = lrclib {
        let saved = LyricsRepository::upsert(
            &state.pool,
            LyricsUpsert {
                track_id: track_id.clone(),
                plain: resp.plain_lyrics,
                synced: resp.synced_lyrics,
                source: LyricsSource::Lrclib,
                lrclib_id: resp.id,
            },
        )
        .await
        .map_err(|e| format!("DB upsert error : {}", e))?;

        return Ok(Some(saved));
    }

    // 6. Repli lyrics.ovh : du texte simple, quand LRCLIB n'a rien eu
    // (absent, en pause ou en panne). Mieux vaut un texte figé que rien.
    let mut panne = matches!(lrclib, Err(_));
    let ovh = if reglage_actif(&state.pool, "lyrics_ovh").await {
        lyrics_ovh_client::fetch_plain(&artist, &title).await
    } else {
        Ok(None)
    };
    match ovh {
        Ok(Some(plain)) => {
            let saved = LyricsRepository::upsert(
                &state.pool,
                LyricsUpsert {
                    track_id: track_id.clone(),
                    plain: Some(plain),
                    synced: None,
                    source: LyricsSource::LyricsOvh,
                    lrclib_id: None,
                },
            )
            .await
            .map_err(|e| format!("DB upsert error : {}", e))?;

            return Ok(Some(saved));
        }
        Ok(None) => {}
        Err(e) => {
            log::warn!("lyrics.ovh fetch failed for {}: {}", path, e);
            panne = true;
        }
    }

    // Une panne de service n'est pas une absence de paroles : on ne la met pas
    // en cache, la prochaine demande réessaiera.
    if panne {
        if let Err(e) = lrclib {
            return Err(e);
        }
        return Err("Aucun service de paroles disponible pour cette demande".to_string());
    }

    let _ = LyricsRepository::mark_not_found(&state.pool, &track_id).await;
    Ok(None)
}

/// Force un re-fetch en supprimant le cache puis en relançant `get_lyrics`.
#[tauri::command]
pub async fn refresh_lyrics(
    state: State<'_, AppState>,
    path: String,
) -> Result<Option<Lyrics>, String> {
    // On a besoin du track_id pour delete : même lookup que get_lyrics
    let file_id = find_file_id_by_path(&state.pool, &path)
        .await
        .map_err(|e| format!("DB error : {}", e))?
        .ok_or_else(|| format!("Fichier introuvable en base : {}", path))?;

    let track = LibraryTrackRepository::find_track_view_by_file_id(&state.pool, &file_id)
        .await
        .map_err(|e| format!("DB error : {}", e))?
        .ok_or_else(|| format!("Track introuvable pour ce fichier : {}", path))?;

    LyricsRepository::delete_by_track_id(&state.pool, &track.id)
        .await
        .map_err(|e| format!("DB delete error : {}", e))?;

    get_lyrics(state, path).await
}

/// Paroles saisies (ou corrigées) à la main par l'utilisateur.
///
/// La base garde la version `manual` — elle prime désormais sur la cascade,
/// le cache étant relu avant toute recherche en ligne. `ecrire_fichier`
/// dépose en plus un `.lrc` à côté du morceau, pour que les paroles suivent
/// la fichier quand on la copie ou qu'un autre lecteur la relise.
#[tauri::command]
pub async fn save_lyrics(
    state: State<'_, AppState>,
    path: String,
    plain: Option<String>,
    synced: Option<String>,
    ecrire_fichier: bool,
) -> Result<Lyrics, String> {
    let file_id = find_file_id_by_path(&state.pool, &path)
        .await
        .map_err(|e| format!("DB error : {}", e))?
        .ok_or_else(|| format!("Fichier introuvable en base : {}", path))?;

    let track = LibraryTrackRepository::find_track_view_by_file_id(&state.pool, &file_id)
        .await
        .map_err(|e| format!("DB error : {}", e))?
        .ok_or_else(|| format!("Track introuvable pour ce fichier : {}", path))?;

    let saved = LyricsRepository::upsert(
        &state.pool,
        LyricsUpsert {
            track_id: track.id.clone(),
            plain,
            synced,
            source: LyricsSource::Manual,
            lrclib_id: None,
        },
    )
    .await
    .map_err(|e| format!("DB upsert error : {}", e))?;

    if ecrire_fichier {
        let contenu = saved.synced.clone().or(saved.plain.clone()).unwrap_or_default();
        if !contenu.trim().is_empty() {
            sidecar::write_sidecar_lrc(&track.path, &contenu)
                .map_err(|e| format!("Écriture .lrc : {e}"))?;
        }
    }

    Ok(saved)
}
