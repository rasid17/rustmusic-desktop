//! Client de l'API publique lyrics.ovh, pour les **paroles simples**.
//!
//! Gratuit et sans clé, mais sans horodatage : le texte seul, ligne à ligne.
//! Il ne remplace pas LRCLIB (`lrclib_client`), qui donne des paroles
//! synchronisées ; il le complète quand LRCLIB n'a rien — mieux vaut un texte
//! figé que rien du tout, et l'interface sait les distinguer.

use serde::Deserialize;
use std::time::Duration;

const LYRICS_OVH_BASE: &str = "https://api.lyrics.ovh/v1";
const USER_AGENT: &str = "RustMusic/0.3.1 (https://rustmusic.dev)";

#[derive(Debug, Deserialize)]
struct OvhResponse {
    lyrics: Option<String>,
}

/// Paroles simples de `title` par `artist` ; `None` si le service n'a rien.
pub async fn fetch_plain(artist: &str, title: &str) -> Result<Option<String>, String> {
    let client = reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|e| format!("HTTP client error: {e}"))?;

    let url = format!(
        "{}/{}/{}",
        LYRICS_OVH_BASE,
        urlencoding::encode(artist),
        urlencoding::encode(title)
    );

    let response = client
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("lyrics.ovh failed: {e}"))?;

    // Le service répond 404 quand il n'a pas la chanson : ce n'est pas une panne.
    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(None);
    }
    if !response.status().is_success() {
        return Err(format!("lyrics.ovh returned {}", response.status()));
    }

    let body: OvhResponse = response
        .json()
        .await
        .map_err(|e| format!("lyrics.ovh JSON parse error: {e}"))?;

    Ok(body.lyrics.filter(|l| !l.trim().is_empty()))
}
