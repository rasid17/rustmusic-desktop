use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::State;

use crate::entity::playlist::playlist::Playlist;
use crate::repository::playlist::playlist_item_repository::PlaylistItemRepository;
use crate::repository::playlist::playlist_repository::PlaylistRepository;
use crate::state::AppState;

/// Informations sur un agent CLI détecté sur le système.
#[derive(Debug, Clone, Serialize)]
pub struct AiAgentInfo {
    pub id: String,
    pub name: String,
    pub description: String,
    pub available: bool,
    pub binary_path: Option<String>,
    pub default_command: String,
}

#[derive(Debug, Deserialize)]
pub struct GenerateAiPlaylistPayload {
    pub profil_id: i64,
    pub library_id: Option<i64>,
    pub agent_id: String,
    pub custom_command: Option<String>,
    pub prompt: String,
    pub max_tracks: Option<usize>,
    pub playlist_name: Option<String>,
    pub color: Option<String>,
    pub icon: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ExtendAiPlaylistPayload {
    pub playlist_id: i64,
    pub profil_id: i64,
    pub library_id: Option<i64>,
    pub agent_id: String,
    pub custom_command: Option<String>,
    pub hint: Option<String>,
    pub add_count: Option<usize>,
}

#[derive(Debug, Deserialize)]
pub struct ReorderAiPlaylistPayload {
    pub playlist_id: i64,
    pub agent_id: String,
    pub custom_command: Option<String>,
    pub instruction: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct AiPlaylistResult {
    pub playlist: Playlist,
    pub track_count: usize,
    pub agent_used: String,
}

#[derive(sqlx::FromRow, Debug, Clone)]
struct CandidateTrack {
    id: String,
    title: String,
    artist: String,
    album: String,
    genre: String,
    year: String,
    play_count: i64,
    favorite: bool,
}

#[derive(sqlx::FromRow, Debug, Clone)]
struct ReorderItem {
    playlist_item_id: i64,
    title: String,
    artist: String,
    album: String,
    genre: String,
    year: String,
}

#[derive(Debug, Deserialize)]
struct AgentJsonResponse {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    tracks: Vec<usize>,
}

/// Liste des répertoires candidats où chercher les binaires CLI (`agy`, `claude`, `gemini`, etc.),
/// y compris les gestionnaires Node (`nvm`, `fnm`) qui ne sont pas toujours dans le `$PATH`
/// d'une session graphique Wayland/X11 lancée depuis un `.desktop`.
fn candidate_bin_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();

    if let Some(path_os) = std::env::var_os("PATH") {
        dirs.extend(std::env::split_paths(&path_os));
    }

    if let Some(home) = dirs::home_dir() {
        dirs.push(home.join(".gemini/antigravity-cli/bin"));
        dirs.push(home.join(".local/bin"));
        dirs.push(home.join(".cargo/bin"));
        dirs.push(home.join(".bun/bin"));
        dirs.push(home.join(".npm-global/bin"));

        // Parcourir ~/.nvm/versions/node/*/bin (du plus récent au plus ancien)
        let nvm_nodes = home.join(".nvm/versions/node");
        if let Ok(entries) = std::fs::read_dir(nvm_nodes) {
            let mut node_bins: Vec<PathBuf> = entries
                .flatten()
                .map(|e| e.path().join("bin"))
                .filter(|p| p.is_dir())
                .collect();
            node_bins.sort();
            node_bins.reverse();
            dirs.extend(node_bins);
        }

        // Parcourir ~/.local/share/fnm/node-versions/*/installation/bin
        let fnm_nodes = home.join(".local/share/fnm/node-versions");
        if let Ok(entries) = std::fs::read_dir(fnm_nodes) {
            let mut fnm_bins: Vec<PathBuf> = entries
                .flatten()
                .map(|e| e.path().join("installation/bin"))
                .filter(|p| p.is_dir())
                .collect();
            fnm_bins.sort();
            fnm_bins.reverse();
            dirs.extend(fnm_bins);
        }
    }

    dirs.push(PathBuf::from("/usr/local/bin"));
    dirs.push(PathBuf::from("/usr/bin"));
    dirs
}

fn find_agent_binary(bin_name: &str) -> Option<PathBuf> {
    for dir in candidate_bin_dirs() {
        let candidate = dir.join(bin_name);
        if candidate.is_file() {
            return Some(candidate);
        }
        #[cfg(target_os = "windows")]
        for ext in ["exe", "cmd", "bat"] {
            let with_ext = dir.join(format!("{bin_name}.{ext}"));
            if with_ext.is_file() {
                return Some(with_ext);
            }
        }
    }
    None
}

/// Construit un `$PATH` enrichi pour que les scripts Node (`#!/usr/bin/env node`)
/// invoqués par `claude`, `gemini`, `agy` ou `codex` trouvent aussi `node`.
fn enriched_path_env() -> String {
    let dirs = candidate_bin_dirs();
    std::env::join_paths(dirs)
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|_| std::env::var("PATH").unwrap_or_default())
}

#[tauri::command]
pub async fn detect_ai_agents() -> Result<Vec<AiAgentInfo>, String> {
    let specs = [
        (
            "agy",
            "Antigravity CLI (agy)",
            "Google Antigravity agent CLI",
            "agy -p",
        ),
        (
            "claude",
            "Claude Code (claude)",
            "Anthropic Claude CLI (`claude -p`)",
            "claude -p",
        ),
        (
            "gemini",
            "Gemini CLI (gemini)",
            "Google Gemini CLI (`gemini -p`)",
            "gemini -p",
        ),
        (
            "codex",
            "Codex CLI (codex)",
            "OpenAI Codex CLI (`codex exec`)",
            "codex exec",
        ),
        (
            "ollama",
            "Ollama (Local LLM)",
            "Local model via Ollama (`ollama run`)",
            "ollama run llama3.2",
        ),
    ];

    let mut agents: Vec<AiAgentInfo> = specs
        .into_iter()
        .map(|(id, name, desc, default_cmd)| {
            let path = find_agent_binary(id);
            AiAgentInfo {
                id: id.to_string(),
                name: name.to_string(),
                description: desc.to_string(),
                available: path.is_some(),
                binary_path: path.map(|p| p.to_string_lossy().into_owned()),
                default_command: default_cmd.to_string(),
            }
        })
        .collect();

    agents.push(AiAgentInfo {
        id: "custom".to_string(),
        name: "Custom CLI Agent".to_string(),
        description: "Commande personnalisée (reçoit le prompt sur stdin)".to_string(),
        available: true,
        binary_path: None,
        default_command: "claude -p".to_string(),
    });

    Ok(agents)
}

/// Pré-filtre et trie jusqu'à `max_catalog` pistes pertinentes et variées pour que le
/// catalogue envoyé à l'agent CLI tienne toujours dans une fenêtre de contexte rapide (~15 Ko).
fn select_catalog_candidates(
    all: Vec<CandidateTrack>,
    prompt: &str,
    max_catalog: usize,
) -> Vec<CandidateTrack> {
    if all.len() <= max_catalog {
        return all;
    }

    let keywords: Vec<String> = prompt
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.len() >= 3)
        .map(|s| s.to_string())
        .collect();

    let mut scored: Vec<(i64, usize, CandidateTrack)> = all
        .into_iter()
        .enumerate()
        .map(|(idx, t)| {
            let mut score = 0i64;
            let hay = format!(
                "{} {} {} {} {}",
                t.title.to_lowercase(),
                t.artist.to_lowercase(),
                t.album.to_lowercase(),
                t.genre.to_lowercase(),
                t.year
            );
            for kw in &keywords {
                if hay.contains(kw) {
                    score += 30;
                }
            }
            if t.favorite {
                score += 8;
            }
            if t.play_count > 0 {
                score += (t.play_count.min(10)) as i64;
            }
            // Pseudo-bruit déterministe pour varier les artistes à score égal
            let hash = ((idx as i64).wrapping_mul(1103515245).wrapping_add(12345) >> 16) & 7;
            (score + hash, idx, t)
        })
        .collect();

    scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));

    // Limiter le nombre de titres d'un même artiste dans l'échantillon pour garantir la variété
    let mut par_artiste = std::collections::HashMap::<String, usize>::new();
    let mut retenus = Vec::with_capacity(max_catalog);
    let mut reserve = Vec::new();

    for (_, _, t) in scored {
        if retenus.len() >= max_catalog {
            break;
        }
        let cle = t.artist.trim().to_lowercase();
        let cnt = par_artiste.entry(cle).or_insert(0);
        if *cnt < 8 {
            *cnt += 1;
            retenus.push(t);
        } else {
            reserve.push(t);
        }
    }

    for t in reserve {
        if retenus.len() >= max_catalog {
            break;
        }
        retenus.push(t);
    }

    retenus
}

fn build_agent_prompt(
    user_prompt: &str,
    max_tracks: usize,
    catalog: &[CandidateTrack],
) -> String {
    let mut lines = String::with_capacity(catalog.len() * 64 + 512);
    lines.push_str(
        "You are an expert music curator generating a playlist from the user's local music library.\n\
         Respond with ONLY a valid JSON object (no markdown fences, no commentary) matching this exact schema:\n\
         {\"name\": \"Playlist Title\", \"description\": \"Brief 1-sentence vibe description\", \"tracks\": [0, 5, 12]}\n\n\
         Rules:\n",
    );
    lines.push_str(&format!(
        "1. Select up to {} track IDs (integers from the first column `id` of the catalog below) that best match the user's request.\n\
         2. Order the tracks for a smooth listening flow (avoid repeating the same artist back-to-back unless requested).\n\
         3. Only use integer IDs present in the catalog below.\n\n\
         User request: \"{}\"\n\n\
         Catalog (id|title|artist|album|genre|year|fav):\n",
        max_tracks,
        user_prompt.trim()
    ));

    for (idx, t) in catalog.iter().enumerate() {
        let clean = |s: &str, max_len: usize| -> String {
            let s = s.replace(['|', '\n', '\r'], " ");
            let trimmed = s.trim();
            if trimmed.chars().count() > max_len {
                trimmed.chars().take(max_len).collect()
            } else {
                trimmed.to_string()
            }
        };
        lines.push_str(&format!(
            "{}|{}|{}|{}|{}|{}|{}\n",
            idx,
            clean(&t.title, 48),
            clean(&t.artist, 36),
            clean(&t.album, 36),
            clean(&t.genre, 24),
            clean(&t.year, 6),
            if t.favorite { "1" } else { "0" }
        ));
    }

    lines
}

fn build_extend_prompt(
    existing_tracks: &[CandidateTrack],
    hint: Option<&str>,
    add_count: usize,
    catalog: &[CandidateTrack],
) -> String {
    let mut lines = String::with_capacity(catalog.len() * 64 + 1024);
    lines.push_str(
        "You are an expert music curator extending an existing playlist with complementary tracks.\n\
         Respond with ONLY a valid JSON object (no markdown fences, no commentary) matching this exact schema:\n\
         {\"tracks\": [0, 4, 9]}\n\n\
         Rules:\n",
    );
    lines.push_str(&format!(
        "1. Select up to {} track IDs (integers from the first column `id` of the catalog below) that best complement and continue the vibe of the current playlist.\n\
         2. Avoid repeating tracks or clashing genres unless requested.\n\
         3. Only use integer IDs present in the catalog below.\n\n",
        add_count
    ));

    if !existing_tracks.is_empty() {
        lines.push_str("Current playlist tracks (seed):\n");
        for (i, t) in existing_tracks.iter().take(30).enumerate() {
            let clean_title = t.title.trim().replace(['\n', '\r', '|'], " ");
            let clean_artist = t.artist.trim().replace(['\n', '\r', '|'], " ");
            let clean_genre = t.genre.trim().replace(['\n', '\r', '|'], " ");
            let genre_part = if clean_genre.is_empty() {
                String::new()
            } else {
                format!(" ({clean_genre})")
            };
            lines.push_str(&format!("{}. {} - {}{}\n", i + 1, clean_title, clean_artist, genre_part));
        }
        lines.push('\n');
    }

    if let Some(h) = hint {
        let h_trimmed = h.trim();
        if !h_trimmed.is_empty() {
            lines.push_str(&format!("User hint / direction: \"{}\"\n\n", h_trimmed));
        }
    }

    lines.push_str("Catalog (id|title|artist|album|genre|year|fav):\n");
    for (idx, t) in catalog.iter().enumerate() {
        let clean = |s: &str, max_len: usize| -> String {
            let s = s.replace(['|', '\n', '\r'], " ");
            let trimmed = s.trim();
            if trimmed.chars().count() > max_len {
                trimmed.chars().take(max_len).collect()
            } else {
                trimmed.to_string()
            }
        };
        lines.push_str(&format!(
            "{}|{}|{}|{}|{}|{}|{}\n",
            idx,
            clean(&t.title, 48),
            clean(&t.artist, 36),
            clean(&t.album, 36),
            clean(&t.genre, 24),
            clean(&t.year, 6),
            if t.favorite { "1" } else { "0" }
        ));
    }

    lines
}

fn build_reorder_prompt(
    items: &[ReorderItem],
    instruction: Option<&str>,
) -> String {
    let mut lines = String::with_capacity(items.len() * 64 + 512);
    lines.push_str(
        "You are an expert music curator and DJ. Reorder the following tracks into an optimal listening progression with a smooth energy, tempo, and genre flow.\n\
         Respond with ONLY a valid JSON object (no markdown fences, no commentary) matching this exact schema:\n\
         {\"tracks\": [2, 0, 1, 3]}\n\n\
         Rules:\n\
         1. The `tracks` array MUST contain all track IDs from 0 to ",
    );
    lines.push_str(&format!("{}", items.len().saturating_sub(1)));
    lines.push_str(
        " in your recommended playback order.\n\
         2. Do NOT leave out any track and do NOT duplicate any ID.\n\
         3. Order tracks for a seamless musical journey (smooth transitions between genres and moods).\n",
    );

    if let Some(inst) = instruction {
        let inst_trimmed = inst.trim();
        if !inst_trimmed.is_empty() {
            lines.push_str(&format!("\nUser specific instruction: \"{}\"\n", inst_trimmed));
        }
    }

    lines.push_str("\nTracks (id|title|artist|album|genre|year):\n");
    for (idx, it) in items.iter().enumerate() {
        let clean = |s: &str, max_len: usize| -> String {
            let s = s.replace(['|', '\n', '\r'], " ");
            let trimmed = s.trim();
            if trimmed.chars().count() > max_len {
                trimmed.chars().take(max_len).collect()
            } else {
                trimmed.to_string()
            }
        };
        lines.push_str(&format!(
            "{}|{}|{}|{}|{}|{}\n",
            idx,
            clean(&it.title, 48),
            clean(&it.artist, 36),
            clean(&it.album, 36),
            clean(&it.genre, 24),
            clean(&it.year, 6),
        ));
    }

    lines
}

fn complete_reorder_indices(chosen_indices: Vec<usize>, total_count: usize) -> Vec<usize> {
    let mut seen = HashSet::new();
    let mut complete = Vec::with_capacity(total_count);
    for idx in chosen_indices {
        if idx < total_count && seen.insert(idx) {
            complete.push(idx);
        }
    }
    for idx in 0..total_count {
        if seen.insert(idx) {
            complete.push(idx);
        }
    }
    complete
}

/// Extrait la réponse JSON de la sortie brute d'un agent CLI (même entourée de ```json ... ``` ou de texte).
fn parse_agent_output(raw: &str, catalog_len: usize, max_tracks: usize) -> Result<(Option<String>, Option<String>, Vec<usize>), String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("L'agent IA a renvoyé une réponse vide.".into());
    }

    // 1. Chercher un objet JSON `{ ... "tracks": [...] ... }`
    if let (Some(start), Some(end)) = (trimmed.find('{'), trimmed.rfind('}')) {
        if start < end {
            let slice = &trimmed[start..=end];
            if let Ok(parsed) = serde_json::from_str::<AgentJsonResponse>(slice) {
                let mut seen = std::collections::HashSet::new();
                let valid: Vec<usize> = parsed
                    .tracks
                    .into_iter()
                    .filter(|&i| i < catalog_len && seen.insert(i))
                    .take(max_tracks)
                    .collect();
                if !valid.is_empty() {
                    return Ok((parsed.name, parsed.description, valid));
                }
            }
        }
    }

    // 2. Fallback : chercher un tableau JSON `[1, 2, 3]`
    if let (Some(start), Some(end)) = (trimmed.find('['), trimmed.rfind(']')) {
        if start < end {
            let slice = &trimmed[start..=end];
            if let Ok(arr) = serde_json::from_str::<Vec<usize>>(slice) {
                let mut seen = std::collections::HashSet::new();
                let valid: Vec<usize> = arr
                    .into_iter()
                    .filter(|&i| i < catalog_len && seen.insert(i))
                    .take(max_tracks)
                    .collect();
                if !valid.is_empty() {
                    return Ok((None, None, valid));
                }
            }
        }
    }

    Err(format!(
        "Impossible d'extraire la liste des pistes depuis la réponse de l'agent : {}",
        trimmed.chars().take(240).collect::<String>()
    ))
}

fn run_cli_agent_blocking(
    agent_id: &str,
    custom_command: Option<&str>,
    full_prompt: &str,
) -> Result<String, String> {
    let path_env = enriched_path_env();

    let execute_cmd = |program: &Path, args: &[&str], pass_stdin: bool| -> Result<String, String> {
        let mut cmd = Command::new(program);
        cmd.args(args)
            .env("PATH", &path_env)
            .stdin(if pass_stdin { Stdio::piped() } else { Stdio::null() })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let mut child = cmd
            .spawn()
            .map_err(|e| format!("Échec du lancement de {} : {e}", program.display()))?;

        if pass_stdin {
            if let Some(mut stdin) = child.stdin.take() {
                let _ = stdin.write_all(full_prompt.as_bytes());
            }
        }

        let output = child
            .wait_with_output()
            .map_err(|e| format!("Erreur d'exécution de l'agent : {e}"))?;

        let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
        let stderr = String::from_utf8_lossy(&output.stderr).into_owned();

        if !output.status.success() && stdout.trim().is_empty() {
            return Err(format!(
                "L'agent {} a échoué (code {:?}) : {}",
                program.display(),
                output.status.code(),
                stderr.trim()
            ));
        }

        Ok(if stdout.trim().is_empty() { stderr } else { stdout })
    };

    match agent_id {
        "claude" => {
            let bin = find_agent_binary("claude")
                .ok_or_else(|| "Agent `claude` introuvable dans le PATH.".to_string())?;
            execute_cmd(&bin, &["-p"], true)
        }
        "gemini" => {
            let bin = find_agent_binary("gemini")
                .ok_or_else(|| "Agent `gemini` introuvable dans le PATH.".to_string())?;
            execute_cmd(&bin, &["-p", "Generate the requested JSON playlist from stdin."], true)
        }
        "agy" => {
            let bin = find_agent_binary("agy")
                .ok_or_else(|| "Agent `agy` introuvable dans le PATH.".to_string())?;
            execute_cmd(&bin, &["-p", full_prompt], false)
                .or_else(|_| execute_cmd(&bin, &[full_prompt], false))
        }
        "codex" => {
            let bin = find_agent_binary("codex")
                .ok_or_else(|| "Agent `codex` introuvable dans le PATH.".to_string())?;
            execute_cmd(&bin, &["exec", full_prompt], false)
        }
        "ollama" => {
            let bin = find_agent_binary("ollama")
                .ok_or_else(|| "Agent `ollama` introuvable dans le PATH.".to_string())?;
            let model = custom_command
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .unwrap_or("llama3.2");
            execute_cmd(&bin, &["run", model], true)
        }
        "custom" => {
            let cmd_str = custom_command
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .ok_or_else(|| "Veuillez saisir une commande pour l'agent personnalisé.".to_string())?;

            #[cfg(target_os = "windows")]
            let (shell, flag) = ("cmd", "/C");
            #[cfg(not(target_os = "windows"))]
            let (shell, flag) = ("sh", "-c");

            let mut child = Command::new(shell)
                .arg(flag)
                .arg(cmd_str)
                .env("PATH", &path_env)
                .env("RUSTMUSIC_PROMPT", full_prompt)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .map_err(|e| format!("Échec de la commande personnalisée : {e}"))?;

            if let Some(mut stdin) = child.stdin.take() {
                let _ = stdin.write_all(full_prompt.as_bytes());
            }

            let output = child
                .wait_with_output()
                .map_err(|e| format!("Erreur d'exécution : {e}"))?;

            let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
            let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
            if !output.status.success() && stdout.trim().is_empty() {
                return Err(format!("La commande a échoué : {}", stderr.trim()));
            }
            Ok(stdout)
        }
        other => Err(format!("Agent IA inconnu : {other}")),
    }
}

#[tauri::command]
pub async fn generate_ai_playlist(
    state: State<'_, AppState>,
    payload: GenerateAiPlaylistPayload,
) -> Result<AiPlaylistResult, String> {
    let prompt_trimmed = payload.prompt.trim().to_string();
    if prompt_trimmed.is_empty() {
        return Err("Veuillez décrire la playlist souhaitée.".into());
    }

    let max_tracks = payload.max_tracks.unwrap_or(25).clamp(5, 100);

    // 1. Charger les pistes actives de la bibliothèque (exclut les dossiers désactivés `is_active = 0`)
    let rows: Vec<CandidateTrack> = sqlx::query_as::<_, CandidateTrack>(
        r#"
        SELECT
            lt.id AS id,
            COALESCE(lt.title, '') AS title,
            COALESCE(a.name, lc.artist, '') AS artist,
            COALESCE(la.title, lc.album, '') AS album,
            COALESCE(lc.genre, la.genre, '') AS genre,
            COALESCE(CAST(la.year AS TEXT), lc.year, '') AS year,
            COALESCE(lt.play_count, 0) AS play_count,
            COALESCE(lt.favorite, 0) AS favorite
        FROM library_tracks lt
        INNER JOIN library_files lf ON lf.id = lt.file_id
            AND NOT EXISTS (SELECT 1 FROM library_dirs ld WHERE ld.id = lf.library_dir_id AND ld.is_active = 0)
        LEFT JOIN library_cache lc ON lc.id = lt.cache_id
        LEFT JOIN library_albums la ON la.id = lt.library_album_id
        LEFT JOIN artists a ON a.id = lt.artist_id
        WHERE lt.library_id IN (SELECT id FROM library WHERE profil_id = ?1)
          AND (?2 IS NULL OR lt.library_id = ?2)
        "#,
    )
    .bind(payload.profil_id)
    .bind(payload.library_id)
    .fetch_all(&state.pool)
    .await
    .map_err(|e| format!("Erreur lecture bibliothèque : {e}"))?;

    if rows.is_empty() {
        return Err("Aucune piste active dans cette bibliothèque.".into());
    }

    let catalog = select_catalog_candidates(rows, &prompt_trimmed, 550);
    let full_prompt = build_agent_prompt(&prompt_trimmed, max_tracks, &catalog);

    let agent_id = payload.agent_id.clone();
    let custom_cmd = payload.custom_command.clone();

    // 2. Exécuter l'agent CLI dans un thread bloquant avec timeout de 120 secondes
    let raw_output = tokio::time::timeout(
        Duration::from_secs(120),
        tauri::async_runtime::spawn_blocking(move || {
            run_cli_agent_blocking(&agent_id, custom_cmd.as_deref(), &full_prompt)
        }),
    )
    .await
    .map_err(|_| "Délai d'attente dépassé (120s) lors de l'appel à l'agent IA.".to_string())?
    .map_err(|e| format!("Erreur interne d'exécution : {e}"))??;

    let (ai_name, ai_desc, chosen_indices) =
        parse_agent_output(&raw_output, catalog.len(), max_tracks)?;

    let final_name = payload
        .playlist_name
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from)
        .or_else(|| ai_name.map(|s| s.trim().to_string()).filter(|s| !s.is_empty()))
        .unwrap_or_else(|| {
            let short: String = prompt_trimmed.chars().take(36).collect();
            format!("AI · {short}")
        });

    let final_desc = ai_desc
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .or_else(|| Some(format!("Générée par {} — {}", payload.agent_id, prompt_trimmed)));

    let color = payload.color.unwrap_or_else(|| "#8b5cf6".to_string());
    let icon = payload.icon.unwrap_or_else(|| "mynaui:sparkles".to_string());

    // 3. Créer la playlist et insérer les morceaux choisis
    let playlist = PlaylistRepository::insert_playlist(
        &state.pool,
        payload.profil_id,
        payload.library_id,
        final_name,
        final_desc,
        color,
        icon,
        None,
        0,
        0,
    )
    .await
    .map_err(|e| format!("Impossible de créer la playlist : {e}"))?;

    for (sort_idx, &cat_idx) in chosen_indices.iter().enumerate() {
        if let Some(track) = catalog.get(cat_idx) {
            let _ = PlaylistItemRepository::insert_playlist_item(
                &state.pool,
                playlist.id,
                track.id.clone(),
                sort_idx as i64,
            )
            .await;
        }
    }

    let _ = PlaylistRepository::recalculate_stats(&state.pool, playlist.id).await;
    let updated_playlist = PlaylistRepository::find_by_id(&state.pool, playlist.id)
        .await
        .ok()
        .flatten()
        .unwrap_or(playlist);

    Ok(AiPlaylistResult {
        playlist: updated_playlist,
        track_count: chosen_indices.len(),
        agent_used: payload.agent_id,
    })
}

#[tauri::command]
pub async fn extend_ai_playlist(
    state: State<'_, AppState>,
    payload: ExtendAiPlaylistPayload,
) -> Result<AiPlaylistResult, String> {
    // 1. Charger les pistes existantes de la playlist
    let existing_tracks: Vec<CandidateTrack> = sqlx::query_as::<_, CandidateTrack>(
        r#"
        SELECT
            lt.id AS id,
            COALESCE(lt.title, '') AS title,
            COALESCE(a.name, lc.artist, '') AS artist,
            COALESCE(la.title, lc.album, '') AS album,
            COALESCE(lc.genre, la.genre, '') AS genre,
            COALESCE(CAST(la.year AS TEXT), lc.year, '') AS year,
            COALESCE(lt.play_count, 0) AS play_count,
            COALESCE(lt.favorite, 0) AS favorite
        FROM playlist_items pi
        INNER JOIN library_tracks lt ON lt.id = pi.library_track_id
        LEFT JOIN library_cache lc ON lc.id = lt.cache_id
        LEFT JOIN library_albums la ON la.id = lt.library_album_id
        LEFT JOIN artists a ON a.id = lt.artist_id
        WHERE pi.playlist_id = ?1
        ORDER BY pi.sort_index ASC, pi.id ASC
        "#,
    )
    .bind(payload.playlist_id)
    .fetch_all(&state.pool)
    .await
    .map_err(|e| format!("Erreur lecture pistes playlist : {e}"))?;

    // 2. Ensemble des IDs de pistes déjà dans la playlist
    let existing_track_ids: HashSet<String> =
        existing_tracks.iter().map(|t| t.id.clone()).collect();

    // 3. Charger les pistes candidates actives de la bibliothèque
    let all_candidates: Vec<CandidateTrack> = sqlx::query_as::<_, CandidateTrack>(
        r#"
        SELECT
            lt.id AS id,
            COALESCE(lt.title, '') AS title,
            COALESCE(a.name, lc.artist, '') AS artist,
            COALESCE(la.title, lc.album, '') AS album,
            COALESCE(lc.genre, la.genre, '') AS genre,
            COALESCE(CAST(la.year AS TEXT), lc.year, '') AS year,
            COALESCE(lt.play_count, 0) AS play_count,
            COALESCE(lt.favorite, 0) AS favorite
        FROM library_tracks lt
        INNER JOIN library_files lf ON lf.id = lt.file_id
            AND NOT EXISTS (SELECT 1 FROM library_dirs ld WHERE ld.id = lf.library_dir_id AND ld.is_active = 0)
        LEFT JOIN library_cache lc ON lc.id = lt.cache_id
        LEFT JOIN library_albums la ON la.id = lt.library_album_id
        LEFT JOIN artists a ON a.id = lt.artist_id
        WHERE lt.library_id IN (SELECT id FROM library WHERE profil_id = ?1)
          AND (?2 IS NULL OR lt.library_id = ?2)
        "#,
    )
    .bind(payload.profil_id)
    .bind(payload.library_id)
    .fetch_all(&state.pool)
    .await
    .map_err(|e| format!("Erreur lecture bibliothèque : {e}"))?;

    let candidates: Vec<CandidateTrack> = all_candidates
        .into_iter()
        .filter(|t| !existing_track_ids.contains(&t.id))
        .collect();

    // 4. Si aucune piste disponible
    if candidates.is_empty() {
        return Err("Aucune piste supplémentaire disponible dans les dossiers actifs.".into());
    }

    // 5. Construire la requête de semence (hint + top artistes / genres)
    let mut seed_parts: Vec<String> = Vec::new();
    if let Some(ref h) = payload.hint {
        let h_trimmed = h.trim();
        if !h_trimmed.is_empty() {
            seed_parts.push(h_trimmed.to_string());
        }
    }

    let mut artist_counts = HashMap::new();
    let mut genre_counts = HashMap::new();
    for t in &existing_tracks {
        let art = t.artist.trim();
        if !art.is_empty() {
            *artist_counts.entry(art).or_insert(0usize) += 1;
        }
        let gen = t.genre.trim();
        if !gen.is_empty() {
            *genre_counts.entry(gen).or_insert(0usize) += 1;
        }
    }
    let mut top_artists: Vec<_> = artist_counts.into_iter().collect();
    top_artists.sort_by(|a, b| b.1.cmp(&a.1));
    let mut top_genres: Vec<_> = genre_counts.into_iter().collect();
    top_genres.sort_by(|a, b| b.1.cmp(&a.1));

    for (art, _) in top_artists.into_iter().take(5) {
        seed_parts.push(art.to_string());
    }
    for (gen, _) in top_genres.into_iter().take(5) {
        seed_parts.push(gen.to_string());
    }
    let seed_query = seed_parts.join(" ");

    let catalog = select_catalog_candidates(candidates, &seed_query, 500);

    // 6. Construire le prompt d'extension
    let add_count = payload.add_count.unwrap_or(10).clamp(1, 50);
    let full_prompt = build_extend_prompt(
        &existing_tracks,
        payload.hint.as_deref(),
        add_count,
        &catalog,
    );

    let agent_id = payload.agent_id.clone();
    let custom_cmd = payload.custom_command.clone();

    // 7. Exécuter l'agent CLI avec timeout de 120s
    let raw_output = tokio::time::timeout(
        Duration::from_secs(120),
        tauri::async_runtime::spawn_blocking(move || {
            run_cli_agent_blocking(&agent_id, custom_cmd.as_deref(), &full_prompt)
        }),
    )
    .await
    .map_err(|_| "Délai d'attente dépassé (120s) lors de l'appel à l'agent IA.".to_string())?
    .map_err(|e| format!("Erreur interne d'exécution : {e}"))??;

    let (_, _, chosen_indices) =
        parse_agent_output(&raw_output, catalog.len(), add_count)?;

    // 8. Insérer les morceaux choisis à la suite
    let base_sort = PlaylistItemRepository::get_next_sort_index(&state.pool, payload.playlist_id)
        .await
        .unwrap_or(0);

    for (sort_idx, &cat_idx) in chosen_indices.iter().enumerate() {
        if let Some(track) = catalog.get(cat_idx) {
            let _ = PlaylistItemRepository::insert_playlist_item(
                &state.pool,
                payload.playlist_id,
                track.id.clone(),
                base_sort + sort_idx as i64,
            )
            .await;
        }
    }

    // 9. Recalculer les statistiques et retourner le résultat
    let _ = PlaylistRepository::recalculate_stats(&state.pool, payload.playlist_id).await;
    let playlist = PlaylistRepository::find_by_id(&state.pool, payload.playlist_id)
        .await
        .map_err(|e| format!("Erreur récupération playlist : {e}"))?
        .ok_or_else(|| "Playlist introuvable.".to_string())?;

    Ok(AiPlaylistResult {
        playlist,
        track_count: chosen_indices.len(),
        agent_used: payload.agent_id,
    })
}

#[tauri::command]
pub async fn reorder_ai_playlist(
    state: State<'_, AppState>,
    payload: ReorderAiPlaylistPayload,
) -> Result<AiPlaylistResult, String> {
    // 1. Charger tous les items de la playlist
    let items: Vec<ReorderItem> = sqlx::query_as::<_, ReorderItem>(
        r#"
        SELECT
            pi.id AS playlist_item_id,
            COALESCE(lt.title, '') AS title,
            COALESCE(a.name, lc.artist, '') AS artist,
            COALESCE(la.title, lc.album, '') AS album,
            COALESCE(lc.genre, la.genre, '') AS genre,
            COALESCE(CAST(la.year AS TEXT), lc.year, '') AS year
        FROM playlist_items pi
        INNER JOIN library_tracks lt ON lt.id = pi.library_track_id
        LEFT JOIN library_cache lc ON lc.id = lt.cache_id
        LEFT JOIN library_albums la ON la.id = lt.library_album_id
        LEFT JOIN artists a ON a.id = lt.artist_id
        WHERE pi.playlist_id = ?1
        ORDER BY pi.sort_index ASC, pi.id ASC
        "#,
    )
    .bind(payload.playlist_id)
    .fetch_all(&state.pool)
    .await
    .map_err(|e| format!("Erreur lecture pistes playlist : {e}"))?;

    // 2. Vérifier au moins 3 pistes
    if items.len() < 3 {
        return Err("La playlist doit contenir au moins 3 pistes pour être réordonnée.".into());
    }

    // 3. Construire le prompt de réordonnancement
    let full_prompt = build_reorder_prompt(&items, payload.instruction.as_deref());

    let agent_id = payload.agent_id.clone();
    let custom_cmd = payload.custom_command.clone();

    // 4. Exécuter l'agent CLI avec timeout de 120s
    let raw_output = tokio::time::timeout(
        Duration::from_secs(120),
        tauri::async_runtime::spawn_blocking(move || {
            run_cli_agent_blocking(&agent_id, custom_cmd.as_deref(), &full_prompt)
        }),
    )
    .await
    .map_err(|_| "Délai d'attente dépassé (120s) lors de l'appel à l'agent IA.".to_string())?
    .map_err(|e| format!("Erreur interne d'exécution : {e}"))??;

    let (_, _, chosen_indices) =
        parse_agent_output(&raw_output, items.len(), items.len())?;

    // 5. Compléter les indices pour garantir qu'aucune piste n'est perdue
    let complete_indices = complete_reorder_indices(chosen_indices, items.len());

    // 6. Mettre à jour sort_index dans une transaction
    let mut tx = state
        .pool
        .begin()
        .await
        .map_err(|e| format!("Erreur début de transaction : {e}"))?;

    for (new_pos, &item_idx) in complete_indices.iter().enumerate() {
        if let Some(item) = items.get(item_idx) {
            sqlx::query("UPDATE playlist_items SET sort_index = ? WHERE id = ?")
                .bind(new_pos as i64)
                .bind(item.playlist_item_id)
                .execute(&mut *tx)
                .await
                .map_err(|e| format!("Erreur mise à jour position piste : {e}"))?;
        }
    }

    tx.commit()
        .await
        .map_err(|e| format!("Erreur validation transaction : {e}"))?;

    // 7. Recalculer les stats et retourner le résultat
    let _ = PlaylistRepository::recalculate_stats(&state.pool, payload.playlist_id).await;
    let playlist = PlaylistRepository::find_by_id(&state.pool, payload.playlist_id)
        .await
        .map_err(|e| format!("Erreur récupération playlist : {e}"))?
        .ok_or_else(|| "Playlist introuvable.".to_string())?;

    Ok(AiPlaylistResult {
        playlist,
        track_count: items.len(),
        agent_used: payload.agent_id,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_clean_or_fenced_json_from_agent() {
        let raw = r#"
Here is your curated playlist:
```json
{
  "name": "Late Night Synth & Indie Pop",
  "description": "Warm synthpop and chill indie tracks without orchestral pieces.",
  "tracks": [3, 1, 4, 1, 99]
}
```
"#;
        let (name, desc, tracks) = parse_agent_output(raw, 10, 5).unwrap();
        assert_eq!(name.as_deref(), Some("Late Night Synth & Indie Pop"));
        assert!(desc.unwrap().contains("Warm synthpop"));
        // Index 99 is out of bounds (< 10) and duplicate `1` is deduplicated
        assert_eq!(tracks, vec![3, 1, 4]);
    }

    #[test]
    fn parses_plain_array_fallback() {
        let raw = "Sure! [0, 2, 5, 2]";
        let (name, _, tracks) = parse_agent_output(raw, 6, 10).unwrap();
        assert!(name.is_none());
        assert_eq!(tracks, vec![0, 2, 5]);
    }

    #[test]
    fn complete_reorder_indices_preserves_all_tracks_and_appends_missing() {
        // Agent returned only [2, 0] for a 5-track playlist (missing 1, 3, 4 and duplicates omitted)
        let chosen = vec![2, 0, 2, 99]; // 99 out-of-bounds, 2 duplicated
        let completed = complete_reorder_indices(chosen, 5);
        assert_eq!(completed, vec![2, 0, 1, 3, 4]);
    }

    #[test]
    fn complete_reorder_indices_handles_empty_selection() {
        let completed = complete_reorder_indices(vec![], 4);
        assert_eq!(completed, vec![0, 1, 2, 3]);
    }
}
