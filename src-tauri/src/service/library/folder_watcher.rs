use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tauri::Emitter;

use crate::repository::library::library_dirs_repository::LibraryDirRepository;

const AUDIO_EXTENSIONS: &[&str] = &[
    "mp3", "flac", "wav", "ogg", "opus", "m4a", "aac", "alac", "wv", "ape", "dsf", "dff", "aiff", "aif", "wma",
];

fn is_audio_file(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| {
            let ext_lower = ext.to_ascii_lowercase();
            AUDIO_EXTENSIONS.iter().any(|&e| e == ext_lower)
        })
        .unwrap_or(false)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirSnapshot {
    pub file_count: u64,
    pub max_mtime_secs: u64,
}

pub fn compute_dir_snapshot(dir_path: &Path) -> DirSnapshot {
    let mut file_count: u64 = 0;
    let mut max_mtime_secs: u64 = 0;

    if let Ok(metadata) = std::fs::metadata(dir_path) {
        if let Ok(mtime) = metadata.modified() {
            if let Ok(duration) = mtime.duration_since(std::time::UNIX_EPOCH) {
                max_mtime_secs = max_mtime_secs.max(duration.as_secs());
            }
        }
    } else {
        return DirSnapshot {
            file_count: 0,
            max_mtime_secs: 0,
        };
    }

    for entry in walkdir::WalkDir::new(dir_path).follow_links(true).into_iter().flatten() {
        let path = entry.path();
        if let Ok(metadata) = entry.metadata() {
            let is_dir = metadata.is_dir();
            let is_audio = metadata.is_file() && is_audio_file(path);

            if is_audio {
                file_count += 1;
            }

            if is_dir || is_audio {
                if let Ok(mtime) = metadata.modified() {
                    if let Ok(duration) = mtime.duration_since(std::time::UNIX_EPOCH) {
                        let secs = duration.as_secs();
                        if secs > max_mtime_secs {
                            max_mtime_secs = secs;
                        }
                    }
                }
            }
        }
    }

    DirSnapshot {
        file_count,
        max_mtime_secs,
    }
}

pub fn start_folder_watcher(app: tauri::AppHandle, pool: sqlx::SqlitePool) {
    tauri::async_runtime::spawn(async move {
        let mut last_synced: HashMap<String, DirSnapshot> = HashMap::new();
        let mut pending_changes: HashMap<String, DirSnapshot> = HashMap::new();

        loop {
            tokio::time::sleep(Duration::from_secs(8)).await;

            let watched_dirs = match LibraryDirRepository::find_watched_active_dirs(&pool).await {
                Ok(dirs) => dirs,
                Err(err) => {
                    log::error!("Folder watcher query error: {}", err);
                    continue;
                }
            };

            for dir in watched_dirs {
                let dir_path_buf = PathBuf::from(&dir.path);
                let current_snapshot = match tokio::task::spawn_blocking(move || {
                    compute_dir_snapshot(&dir_path_buf)
                })
                .await
                {
                    Ok(snapshot) => snapshot,
                    Err(e) => {
                        log::error!("Folder watcher task error for dir {}: {}", dir.path, e);
                        continue;
                    }
                };

                let last = match last_synced.get(&dir.id) {
                    Some(s) => s,
                    None => {
                        last_synced.insert(dir.id.clone(), current_snapshot);
                        continue;
                    }
                };

                if &current_snapshot != last {
                    if pending_changes.get(&dir.id) == Some(&current_snapshot) {
                        let Some(jeton) = crate::service::library::library_service::IMPORT.start() else {
                            continue;
                        };

                        pending_changes.remove(&dir.id);
                        last_synced.insert(dir.id.clone(), current_snapshot);

                        if let Err(e) = crate::service::library::library_service::save_dir_to_library(
                            app.clone(),
                            &pool,
                            dir.library_id,
                            dir.path.clone(),
                            &jeton,
                        )
                        .await
                        {
                            log::warn!("Folder watcher auto-import failed for {}: {}", dir.path, e);
                        }

                        let _ = app.emit(
                            "library-auto-synced",
                            serde_json::json!({
                                "libraryId": dir.library_id,
                                "dirId": dir.id,
                                "path": dir.path
                            }),
                        );
                    } else {
                        pending_changes.insert(dir.id.clone(), current_snapshot);
                    }
                } else {
                    pending_changes.remove(&dir.id);
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn test_compute_dir_snapshot() {
        let temp_dir = std::env::temp_dir().join(format!("test_watcher_{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&temp_dir).expect("failed to create temp dir");

        let snap_empty = compute_dir_snapshot(&temp_dir);
        assert_eq!(snap_empty.file_count, 0);

        let txt_file = temp_dir.join("notes.txt");
        fs::write(&txt_file, b"sample text").unwrap();
        let snap_txt = compute_dir_snapshot(&temp_dir);
        assert_eq!(snap_txt.file_count, 0);

        let flac_file = temp_dir.join("song.flac");
        fs::write(&flac_file, b"sample flac content").unwrap();
        let snap_audio = compute_dir_snapshot(&temp_dir);
        assert_eq!(snap_audio.file_count, 1);
        assert!(snap_audio.max_mtime_secs > 0);

        let sub_dir = temp_dir.join("sub");
        fs::create_dir_all(&sub_dir).unwrap();
        let mp3_file = sub_dir.join("track.mp3");
        fs::write(&mp3_file, b"sample mp3 content").unwrap();

        let snap_sub = compute_dir_snapshot(&temp_dir);
        assert_eq!(snap_sub.file_count, 2);
        assert!(snap_sub.max_mtime_secs >= snap_audio.max_mtime_secs);

        let _ = fs::remove_dir_all(&temp_dir);
    }
}
