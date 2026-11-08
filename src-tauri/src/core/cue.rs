use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq)]
pub struct CueTrack {
    pub track_no: u16,
    pub title: Option<String>,
    pub performer: Option<String>,
    pub start_secs: f64,
    pub end_secs: Option<f64>,
    pub audio_file: PathBuf,
    pub cue_file: PathBuf,
    pub album_title: Option<String>,
    pub album_performer: Option<String>,
    pub genre: Option<String>,
    pub year: Option<u16>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CueUriInfo {
    pub real_path: PathBuf,
    pub cue_path: Option<PathBuf>,
    pub track_no: u16,
    pub start_secs: f64,
    pub end_secs: Option<f64>,
}

fn extract_string_value(rest: &str) -> String {
    let trimmed = rest.trim();
    if trimmed.starts_with('"') {
        if let Some(end_idx) = trimmed[1..].find('"') {
            return trimmed[1..1 + end_idx].to_string();
        }
    }
    trimmed.trim_matches('"').to_string()
}

fn parse_file_line(line: &str) -> Option<String> {
    let rem = line.strip_prefix("FILE")?.trim();
    if rem.starts_with('"') {
        let end_idx = rem[1..].find('"')?;
        Some(rem[1..1 + end_idx].to_string())
    } else {
        let tokens: Vec<&str> = rem.split_whitespace().collect();
        if tokens.len() > 1 {
            Some(tokens[..tokens.len() - 1].join(" "))
        } else if !tokens.is_empty() {
            Some(tokens[0].to_string())
        } else {
            None
        }
    }
}

fn parse_index_time(time_str: &str) -> Option<f64> {
    let parts: Vec<&str> = time_str.split(':').collect();
    if parts.len() != 3 {
        return None;
    }
    let mm: f64 = parts[0].parse().ok()?;
    let ss: f64 = parts[1].parse().ok()?;
    let ff: f64 = parts[2].parse().ok()?;
    Some((mm * 60.0) + ss + (ff / 75.0))
}

fn parse_year(val: &str) -> Option<u16> {
    let digits: String = val.chars().filter(|c| c.is_ascii_digit()).take(4).collect();
    digits.parse::<u16>().ok()
}

struct PartialTrack {
    track_no: u16,
    title: Option<String>,
    performer: Option<String>,
    start_secs: Option<f64>,
    audio_file: PathBuf,
}

pub fn parse_cue_content(content: &str, cue_path: &Path) -> Vec<CueTrack> {
    let mut album_genre: Option<String> = None;
    let mut album_year: Option<u16> = None;
    let mut album_performer: Option<String> = None;
    let mut album_title: Option<String> = None;
    let mut current_audio_file: Option<PathBuf> = None;

    let mut current_track: Option<PartialTrack> = None;
    let mut partial_tracks: Vec<PartialTrack> = Vec::new();

    let cue_parent = cue_path.parent();

    for raw_line in content.lines() {
        let line = raw_line.trim();
        if line.is_empty() {
            continue;
        }

        let upper = line.to_ascii_uppercase();

        if upper.starts_with("REM") {
            let rem_rest = line[3..].trim();
            let rem_upper = rem_rest.to_ascii_uppercase();
            if rem_upper.starts_with("GENRE") {
                album_genre = Some(extract_string_value(&rem_rest[5..]));
            } else if rem_upper.starts_with("DATE") {
                album_year = parse_year(rem_rest[4..].trim());
            } else if rem_upper.starts_with("YEAR") {
                album_year = parse_year(rem_rest[4..].trim());
            }
        } else if upper.starts_with("PERFORMER") {
            let val = extract_string_value(&line[9..]);
            if let Some(ref mut trk) = current_track {
                trk.performer = Some(val);
            } else {
                album_performer = Some(val);
            }
        } else if upper.starts_with("TITLE") {
            let val = extract_string_value(&line[5..]);
            if let Some(ref mut trk) = current_track {
                trk.title = Some(val);
            } else {
                album_title = Some(val);
            }
        } else if upper.starts_with("FILE") {
            if let Some(fname) = parse_file_line(line) {
                let resolved = if let Some(parent) = cue_parent {
                    parent.join(&fname)
                } else {
                    PathBuf::from(&fname)
                };
                current_audio_file = Some(resolved);
            }
        } else if upper.starts_with("TRACK") {
            if let Some(prev) = current_track.take() {
                partial_tracks.push(prev);
            }

            let parts: Vec<&str> = line.split_whitespace().collect();
            let track_no = parts.get(1).and_then(|n| n.parse::<u16>().ok()).unwrap_or(1);
            let audio_file = current_audio_file.clone().unwrap_or_else(|| {
                cue_parent
                    .map(|p| p.join("unknown"))
                    .unwrap_or_else(|| PathBuf::from("unknown"))
            });

            current_track = Some(PartialTrack {
                track_no,
                title: None,
                performer: None,
                start_secs: None,
                audio_file,
            });
        } else if upper.starts_with("INDEX") {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 3 {
                let idx_num = parts[1];
                if (idx_num == "01" || idx_num == "1") && current_track.is_some() {
                    if let Some(secs) = parse_index_time(parts[2]) {
                        if let Some(ref mut trk) = current_track {
                            trk.start_secs = Some(secs);
                        }
                    }
                }
            }
        }
    }

    if let Some(last) = current_track.take() {
        partial_tracks.push(last);
    }

    let mut tracks: Vec<CueTrack> = partial_tracks
        .into_iter()
        .map(|pt| CueTrack {
            track_no: pt.track_no,
            title: pt.title,
            performer: pt.performer,
            start_secs: pt.start_secs.unwrap_or(0.0),
            end_secs: None,
            audio_file: pt.audio_file,
            cue_file: cue_path.to_path_buf(),
            album_title: album_title.clone(),
            album_performer: album_performer.clone(),
            genre: album_genre.clone(),
            year: album_year,
        })
        .collect();

    let total = tracks.len();
    for i in 0..total {
        if i + 1 < total && tracks[i].audio_file == tracks[i + 1].audio_file {
            tracks[i].end_secs = Some(tracks[i + 1].start_secs);
        } else {
            tracks[i].end_secs = None;
        }
    }

    tracks
}

pub fn parse_cue_file(cue_path: &Path) -> Vec<CueTrack> {
    let bytes = match std::fs::read(cue_path) {
        Ok(b) => b,
        Err(_) => return Vec::new(),
    };
    let content_lossy = String::from_utf8_lossy(&bytes);
    let content = content_lossy.strip_prefix('\u{feff}').unwrap_or(&content_lossy);
    parse_cue_content(content, cue_path)
}

pub fn build_cue_uri(t: &CueTrack) -> String {
    let end_str = t.end_secs.map(|e| format!("{:.3}", e)).unwrap_or_default();
    format!(
        "cue://{}#track={}&start={:.3}&end={}&cue={}",
        t.audio_file.to_string_lossy(),
        t.track_no,
        t.start_secs,
        end_str,
        t.cue_file.to_string_lossy()
    )
}

pub fn parse_cue_uri(uri: &str) -> Option<CueUriInfo> {
    let rest = uri.strip_prefix("cue://")?;
    let (real_path_str, fragment) = rest.split_once('#')?;
    let real_path = PathBuf::from(real_path_str);

    let mut track_no: u16 = 1;
    let mut start_secs: f64 = 0.0;
    let mut end_secs: Option<f64> = None;
    let mut cue_path: Option<PathBuf> = None;

    for part in fragment.split('&') {
        if let Some((k, v)) = part.split_once('=') {
            match k {
                "track" => {
                    if let Ok(num) = v.parse::<u16>() {
                        track_no = num;
                    }
                }
                "start" => {
                    if let Ok(s) = v.parse::<f64>() {
                        start_secs = s;
                    }
                }
                "end" => {
                    if !v.is_empty() {
                        end_secs = v.parse::<f64>().ok();
                    }
                }
                "cue" => {
                    if !v.is_empty() {
                        cue_path = Some(PathBuf::from(v));
                    }
                }
                _ => {}
            }
        }
    }

    Some(CueUriInfo {
        real_path,
        cue_path,
        track_no,
        start_secs,
        end_secs,
    })
}

pub fn resolve_cue_track_metadata(info: &CueUriInfo) -> Option<CueTrack> {
    let cue_file = info.cue_path.as_ref()?;
    let tracks = parse_cue_file(cue_file);
    tracks.into_iter().find(|t| t.track_no == info.track_no)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_cue_content() {
        let cue_text = r#"
REM GENRE "Alternative Rock"
REM DATE 1997
PERFORMER "Radiohead"
TITLE "OK Computer"
FILE "Radiohead - OK Computer.flac" WAVE
  TRACK 01 AUDIO
    TITLE "Airbag"
    PERFORMER "Radiohead"
    INDEX 01 00:00:00
  TRACK 02 AUDIO
    TITLE "Paranoid Android"
    PERFORMER "Radiohead"
    INDEX 01 04:44:00
  TRACK 03 AUDIO
    TITLE "Subterranean Homesick Alien"
    PERFORMER "Radiohead"
    INDEX 01 11:11:37
"#;
        let cue_path = Path::new("/music/Radiohead - OK Computer.cue");
        let tracks = parse_cue_content(cue_text, cue_path);

        assert_eq!(tracks.len(), 3);

        // Track 1
        assert_eq!(tracks[0].track_no, 1);
        assert_eq!(tracks[0].title.as_deref(), Some("Airbag"));
        assert_eq!(tracks[0].performer.as_deref(), Some("Radiohead"));
        assert_eq!(tracks[0].album_title.as_deref(), Some("OK Computer"));
        assert_eq!(tracks[0].album_performer.as_deref(), Some("Radiohead"));
        assert_eq!(tracks[0].genre.as_deref(), Some("Alternative Rock"));
        assert_eq!(tracks[0].year, Some(1997));
        assert_eq!(tracks[0].audio_file, PathBuf::from("/music/Radiohead - OK Computer.flac"));
        assert_eq!(tracks[0].start_secs, 0.0);
        assert_eq!(tracks[0].end_secs, Some(4.0 * 60.0 + 44.0)); // 284.0

        // Track 2
        assert_eq!(tracks[1].track_no, 2);
        assert_eq!(tracks[1].title.as_deref(), Some("Paranoid Android"));
        assert_eq!(tracks[1].start_secs, 284.0);
        let trk3_start = (11.0 * 60.0) + 11.0 + (37.0 / 75.0);
        assert_eq!(tracks[1].end_secs, Some(trk3_start));

        // Track 3
        assert_eq!(tracks[2].track_no, 3);
        assert_eq!(tracks[2].title.as_deref(), Some("Subterranean Homesick Alien"));
        assert_eq!(tracks[2].start_secs, trk3_start);
        assert_eq!(tracks[2].end_secs, None);
    }

    #[test]
    fn test_cue_uri_roundtrip() {
        let track = CueTrack {
            track_no: 2,
            title: Some("Paranoid Android".to_string()),
            performer: Some("Radiohead".to_string()),
            start_secs: 284.0,
            end_secs: Some(671.493),
            audio_file: PathBuf::from("/music/album.flac"),
            cue_file: PathBuf::from("/music/album.cue"),
            album_title: Some("OK Computer".to_string()),
            album_performer: Some("Radiohead".to_string()),
            genre: Some("Rock".to_string()),
            year: Some(1997),
        };

        let uri = build_cue_uri(&track);
        assert_eq!(
            uri,
            "cue:///music/album.flac#track=2&start=284.000&end=671.493&cue=/music/album.cue"
        );

        let parsed = parse_cue_uri(&uri).expect("failed to parse cue uri");
        assert_eq!(parsed.real_path, PathBuf::from("/music/album.flac"));
        assert_eq!(parsed.cue_path, Some(PathBuf::from("/music/album.cue")));
        assert_eq!(parsed.track_no, 2);
        assert_eq!(parsed.start_secs, 284.0);
        assert_eq!(parsed.end_secs, Some(671.493));

        // Test with end_secs = None
        let mut track_no_end = track.clone();
        track_no_end.end_secs = None;
        let uri_no_end = build_cue_uri(&track_no_end);
        assert_eq!(
            uri_no_end,
            "cue:///music/album.flac#track=2&start=284.000&end=&cue=/music/album.cue"
        );
        let parsed_no_end = parse_cue_uri(&uri_no_end).expect("failed to parse no end cue uri");
        assert_eq!(parsed_no_end.end_secs, None);
    }
}
