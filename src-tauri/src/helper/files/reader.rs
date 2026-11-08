use std::{collections::HashSet, fs::{self}, path::{Path, PathBuf}};

const AUDIO_EXTENSIONS: &[&str] = &[
    "mp3", "flac", "wav", "aiff", "aif",
    "ogg", "opus", "m4a", "aac",
    "dsf", "dff",
];

// Insensible à la casse : « .FLAC » ou « .Mp3 » (vieux rips) étaient ignorés.
fn is_audio(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| AUDIO_EXTENSIONS.contains(&ext.to_ascii_lowercase().as_str()))
}

pub fn read_dir_deep(
    directory: &str,
    files: &mut Vec<PathBuf>
) {
    read_dir_deep_with_progress(directory, files, &mut |_| true);
}

/// `on_progress` reçoit le nombre de fichiers trouvés ; faux arrête le parcours.
/// Sans récursion, et chaque dossier n'est lu qu'une fois : une jonction qui boucle
/// (vieux disque système, « Application Data ») ne fait plus tourner le scan sans fin.
pub fn read_dir_deep_with_progress(
    directory: &str,
    files: &mut Vec<PathBuf>,
    on_progress: &mut dyn FnMut(usize) -> bool,
) -> bool {
    let mut visited: HashSet<PathBuf> = HashSet::new();
    let mut stack: Vec<PathBuf> = vec![PathBuf::from(directory)];

    while let Some(dir) = stack.pop() {
        let real = fs::canonicalize(&dir).unwrap_or_else(|_| dir.clone());
        if !visited.insert(real) {
            continue;
        }
        let entries = match fs::read_dir(&dir) {
            Ok(e) => e,
            Err(e) => {
                log::warn!("Impossible de lire le dir {:?}: {}", dir, e);
                continue;
            }
        };

        let mut subdirs: Vec<PathBuf> = Vec::new();
        let mut cue_files: Vec<PathBuf> = Vec::new();
        let mut audio_candidates: Vec<PathBuf> = Vec::new();

        for entry in entries.flatten() {
            let path: PathBuf = entry.path();
            let Ok(kind) = entry.file_type() else { continue };
            // Un lien vers un dossier est suivi ; `visited` coupe les boucles.
            let is_dir_entry = kind.is_dir() || (kind.is_symlink() && path.is_dir());

            if is_dir_entry {
                if !is_system_dir(&path) {
                    subdirs.push(path);
                }
            } else {
                let is_cue = path
                    .extension()
                    .and_then(|ext| ext.to_str())
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("cue"));

                if is_cue {
                    cue_files.push(path);
                } else if is_audio(&path) {
                    audio_candidates.push(path);
                }
            }
        }

        let mut cue_covered_files: HashSet<PathBuf> = HashSet::new();
        for cue_path in cue_files {
            let tracks = crate::core::cue::parse_cue_file(&cue_path);
            for cue_track in tracks {
                if cue_track.audio_file.exists() {
                    cue_covered_files.insert(cue_track.audio_file.clone());
                    if let Ok(canon) = fs::canonicalize(&cue_track.audio_file) {
                        cue_covered_files.insert(canon);
                    }
                    let uri = crate::core::cue::build_cue_uri(&cue_track);
                    files.push(PathBuf::from(uri));
                    if !on_progress(files.len()) {
                        return false;
                    }
                }
            }
        }

        for audio_path in audio_candidates {
            let is_covered = cue_covered_files.contains(&audio_path)
                || fs::canonicalize(&audio_path)
                    .map(|c| cue_covered_files.contains(&c))
                    .unwrap_or(false);
            if !is_covered {
                files.push(audio_path);
                if !on_progress(files.len()) {
                    return false;
                }
            }
        }
        // Pile : à l'envers pour garder l'ordre de lecture.
        stack.extend(subdirs.into_iter().rev());
    }

    on_progress(files.len())
}

/// Corbeille et données système Windows : jamais de musique, parfois des milliers d'entrées.
fn is_system_dir(path: &Path) -> bool {
    path.file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n.eq_ignore_ascii_case("$RECYCLE.BIN") || n.eq_ignore_ascii_case("System Volume Information"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_dir(nom: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("rustmusic-{nom}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(d.join("A/B")).unwrap();
        fs::write(d.join("A/un.flac"), b"").unwrap();
        fs::write(d.join("A/B/deux.MP3"), b"").unwrap();
        fs::write(d.join("A/note.txt"), b"").unwrap();
        fs::create_dir_all(d.join("$RECYCLE.BIN")).unwrap();
        fs::write(d.join("$RECYCLE.BIN/efface.flac"), b"").unwrap();
        d
    }

    #[test]
    fn finds_audio_and_skips_recycle_bin() {
        let d = test_dir("parcours");
        let mut files = Vec::new();
        read_dir_deep(d.to_str().unwrap(), &mut files);
        let mut names: Vec<_> = files.iter().map(|f| f.file_name().unwrap().to_string_lossy().to_string()).collect();
        names.sort();
        assert_eq!(names, ["deux.MP3", "un.flac"]);
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn finds_cue_and_skips_covered_audio() {
        let d = std::env::temp_dir().join(format!("rustmusic-cue-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        fs::write(d.join("album.flac"), b"dummy audio content").unwrap();
        fs::write(d.join("standalone.mp3"), b"dummy audio content").unwrap();
        let cue_content = r#"
TITLE "Test Album"
PERFORMER "Test Artist"
FILE "album.flac" WAVE
  TRACK 01 AUDIO
    TITLE "Track 1"
    INDEX 01 00:00:00
  TRACK 02 AUDIO
    TITLE "Track 2"
    INDEX 01 03:00:00
"#;
        fs::write(d.join("album.cue"), cue_content).unwrap();

        let mut files = Vec::new();
        read_dir_deep(d.to_str().unwrap(), &mut files);

        // album.flac should be skipped because it is covered by album.cue
        // standalone.mp3 should be included
        // 2 cue tracks should be included
        assert_eq!(files.len(), 3);
        let cue_uris: Vec<_> = files.iter().filter(|f| f.to_string_lossy().starts_with("cue://")).collect();
        assert_eq!(cue_uris.len(), 2);
        let non_cue: Vec<_> = files.iter().filter(|f| !f.to_string_lossy().starts_with("cue://")).collect();
        assert_eq!(non_cue.len(), 1);
        assert_eq!(non_cue[0].file_name().unwrap(), "standalone.mp3");

        let _ = fs::remove_dir_all(&d);
    }

    #[cfg(windows)]
    #[test]
    fn looping_junction_does_not_recurse_forever() {
        let d = test_dir("boucle");
        // A/B/retour → A : sans garde, A/B/retour/B/retour/… à l'infini.
        let ok = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(d.join("A").join("B").join("retour"))
            .arg(d.join("A"))
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        assert!(ok, "mklink /J indisponible");
        let mut files = Vec::new();
        read_dir_deep(d.to_str().unwrap(), &mut files);
        assert_eq!(files.len(), 2);
        let _ = fs::remove_dir(d.join("A").join("B").join("retour"));
        let _ = fs::remove_dir_all(&d);
    }
}
