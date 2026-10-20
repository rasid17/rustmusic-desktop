use serde::Deserialize;
use std::time::Duration;
use tokio::sync::Mutex;
use tokio::time::Instant;

const LRCLIB_GET_URL: &str = "https://lrclib.net/api/get";
const LRCLIB_SEARCH_URL: &str = "https://lrclib.net/api/search";
const USER_AGENT: &str = "RustMusic/0.1.4 (https://rustmusic.dev)";

/// LRCLIB demande une requête par seconde au maximum. Passer outre vaut des
/// 429, et donc des paroles absentes pour tout le monde pendant un moment.
const MIN_INTERVAL: Duration = Duration::from_millis(1_100);
/// Après un rejet (429) ou une panne (5xx), on ne re-tente plus pendant ce délai.
const COOLDOWN_REJET: Duration = Duration::from_secs(60);
const COOLDOWN_PANNE: Duration = Duration::from_secs(30);

static LAST_CALL: Mutex<Option<Instant>> = Mutex::const_new(None);
static COOLDOWN_UNTIL: Mutex<Option<Instant>> = Mutex::const_new(None);

/// Attend si nécessaire pour tenir le débit d'une requête par seconde.
async fn throttle() {
    let mut last = LAST_CALL.lock().await;
    if let Some(previous) = *last {
        let elapsed = previous.elapsed();
        if elapsed < MIN_INTERVAL {
            tokio::time::sleep(MIN_INTERVAL - elapsed).await;
        }
    }
    *last = Some(Instant::now());
}

/// Le service a rejeté ou est tombé : on le laisse respirer.
async fn mettre_en_pause(duree: Duration) {
    *COOLDOWN_UNTIL.lock().await = Some(Instant::now() + duree);
}

/// `Err` tant que la pause est en cours — la cascade passera alors à la suite
/// sans marquer le morceau comme « sans paroles ».
async fn verifier_pause() -> Result<(), String> {
    let garde = COOLDOWN_UNTIL.lock().await;
    if let Some(until) = *garde {
        let now = Instant::now();
        if now < until {
            return Err(format!("LRCLIB en pause ({} s restantes)", (until - now).as_secs()));
        }
    }
    Ok(())
}

/// Réponse brute renvoyée par LRCLIB.
#[derive(Debug, Deserialize, Clone)]
pub struct LrclibResponse {
    pub id: Option<i64>,
    #[serde(rename = "plainLyrics")]
    pub plain_lyrics: Option<String>,
    #[serde(rename = "syncedLyrics")]
    pub synced_lyrics: Option<String>,
    pub instrumental: Option<bool>,
    pub duration: Option<f64>,
}

/// Récupère les paroles d'un morceau sur LRCLIB avec stratégie en cascade :
/// 1. /api/get strict (artist + title + album + duration) — match exact rapide
/// 2. Si /get a renvoyé un record SANS synced, /api/search pour trouver une
///    version synchronisée parmi les multiples records
/// 3. /api/search direct si /get a renvoyé 404
///
/// Retourne le meilleur match disponible (synced > plain), ou None si rien.
pub async fn fetch_lyrics(
    artist: &str,
    title: &str,
    album: Option<&str>,
    duration: u32,
) -> Result<Option<LrclibResponse>, String> {

    // En pause après un rejet : la cascade ira chercher ailleurs sans perdre
    // la trace du morceau (il n'est pas « sans paroles », juste non demandé).
    verifier_pause().await?;

    let client = reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| format!("HTTP client error: {}", e))?;

    // ─── 1. Tentative /api/get strict ───
    let get_result = try_get(&client, artist, title, album, duration).await?;

    // Si on a déjà du synced, on ne va pas plus loin
    if let Some(ref resp) = get_result {
        if resp.synced_lyrics.as_ref().map(|s| !s.is_empty()).unwrap_or(false) {
            return Ok(get_result);
        }
    }

    // ─── 2. Fallback /api/search pour trouver une version avec synced ───
    let search_results = try_search(&client, artist, title).await?;

    // Cherche le meilleur match : préfère les versions avec syncedLyrics
    // et durée proche de la nôtre
    let best = pick_best(&search_results, duration);

    // Si on a trouvé une meilleure version (avec synced) → on la prend
    if let Some(found) = best {
        if found.synced_lyrics.as_ref().map(|s| !s.is_empty()).unwrap_or(false) {
            return Ok(Some(found));
        }
    }

    // Sinon on garde ce qu'on avait au /get (plain seulement)
    Ok(get_result)
}

async fn try_get(
    client: &reqwest::Client,
    artist: &str,
    title: &str,
    album: Option<&str>,
    duration: u32,
) -> Result<Option<LrclibResponse>, String> {

    let mut url = format!(
        "{}?artist_name={}&track_name={}",
        LRCLIB_GET_URL,
        urlencoding::encode(artist),
        urlencoding::encode(title),
    );

    if let Some(a) = album {
        if !a.is_empty() {
            url.push_str(&format!("&album_name={}", urlencoding::encode(a)));
        }
    }

    if duration > 0 {
        url.push_str(&format!("&duration={}", duration));
    }

    throttle().await;

    let response = client
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("LRCLIB /get failed: {}", e))?;

    let status = response.status();

    if status == reqwest::StatusCode::NOT_FOUND {
        return Ok(None);
    }
    if !status.is_success() {
        if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            mettre_en_pause(COOLDOWN_REJET).await;
        } else if status.is_server_error() {
            mettre_en_pause(COOLDOWN_PANNE).await;
        }
        return Err(format!("LRCLIB /get returned {}", status));
    }

    let body: LrclibResponse = response
        .json()
        .await
        .map_err(|e| format!("LRCLIB /get JSON parse error: {}", e))?;

    if body.plain_lyrics.is_none() && body.synced_lyrics.is_none() {
        return Ok(None);
    }

    Ok(Some(body))
}

async fn try_search(
    client: &reqwest::Client,
    artist: &str,
    title: &str,
) -> Result<Vec<LrclibResponse>, String> {

    let url = format!(
        "{}?artist_name={}&track_name={}",
        LRCLIB_SEARCH_URL,
        urlencoding::encode(artist),
        urlencoding::encode(title),
    );

    throttle().await;

    let response = client
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("LRCLIB /search failed: {}", e))?;

    if !response.status().is_success() {
        let status = response.status();
        if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            mettre_en_pause(COOLDOWN_REJET).await;
        } else if status.is_server_error() {
            mettre_en_pause(COOLDOWN_PANNE).await;
        }
        return Ok(vec![]);
    }

    let results: Vec<LrclibResponse> = response
        .json()
        .await
        .map_err(|e| format!("LRCLIB /search JSON parse error: {}", e))?;

    Ok(results)
}

/// Choisit le meilleur record parmi les résultats de search.
/// Préfère :
/// 1. Ceux avec syncedLyrics non-vide
/// 2. Durée la plus proche de la nôtre (tolérance 5s)
fn pick_best(results: &[LrclibResponse], target_duration: u32) -> Option<LrclibResponse> {
    if results.is_empty() {
        return None;
    }

    // Filtrer ceux qui ont du synced d'abord
    let with_synced: Vec<&LrclibResponse> = results
        .iter()
        .filter(|r| r.synced_lyrics.as_ref().map(|s| !s.is_empty()).unwrap_or(false))
        .collect();

    let pool = if !with_synced.is_empty() {
        with_synced
    } else {
        results.iter().collect()
    };

    if target_duration == 0 {
        return Some(pool[0].clone());
    }

    let target = target_duration as f64;
    pool.into_iter()
        .min_by(|a, b| {
            let da = (a.duration.unwrap_or(0.0) - target).abs();
            let db = (b.duration.unwrap_or(0.0) - target).abs();
            da.partial_cmp(&db).unwrap_or(std::cmp::Ordering::Equal)
        })
        .cloned()
}
