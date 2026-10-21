use serde::Serialize;
use tauri::State;

use crate::core::smart_playlist::{engine, field, rules};
use crate::mapper::playlist::playlist::playlist_track_view::PlaylistTrackView;
use crate::state::AppState;

/// Un opérateur, tel que l'interface le propose.
#[derive(Debug, Serialize)]
pub struct OpInfo {
    pub key: &'static str,
    pub label: &'static str,
    /// Nombre de valeurs à saisir : 0 pour « est vide », 2 pour « entre ».
    pub arity: u8,
}

/// Le vocabulaire complet des règles, servi à l'interface.
///
/// Le décrire ici plutôt que de le recopier côté interface évite qu'un champ
/// ajouté en Rust reste invisible, ou qu'un opérateur proposé n'existe pas.
#[derive(Debug, Serialize)]
pub struct RuleVocabulary {
    pub fields: Vec<field::FieldInfo>,
    /// Opérateurs par nature de champ : `text`, `number`, `date`, `bool`.
    pub operators: std::collections::BTreeMap<String, Vec<OpInfo>>,
}

fn arity(op: rules::Op) -> u8 {
    use rules::Op::*;
    match op {
        IsEmpty | IsNotEmpty | IsTrue | IsFalse => 0,
        Between => 2,
        _ => 1,
    }
}

fn op_key(op: rules::Op) -> &'static str {
    use rules::Op::*;
    match op {
        Contains => "contains",
        NotContains => "not_contains",
        Is => "is",
        IsNot => "is_not",
        StartsWith => "starts_with",
        EndsWith => "ends_with",
        IsEmpty => "is_empty",
        IsNotEmpty => "is_not_empty",
        Eq => "eq",
        Neq => "neq",
        Gt => "gt",
        Gte => "gte",
        Lt => "lt",
        Lte => "lte",
        Between => "between",
        Before => "before",
        After => "after",
        InLast => "in_last",
        NotInLast => "not_in_last",
        IsTrue => "is_true",
        IsFalse => "is_false",
        InTopPlayed => "in_top_played",
    }
}

#[tauri::command]
pub fn get_rule_vocabulary() -> RuleVocabulary {
    use field::FieldKind::*;

    let mut operators = std::collections::BTreeMap::new();
    for (nom, kind) in [("text", Text), ("number", Number), ("date", Date), ("bool", Bool)] {
        operators.insert(
            nom.to_string(),
            rules::Op::for_kind(kind)
                .iter()
                .map(|op| OpInfo {
                    key: op_key(*op),
                    label: op.label(),
                    arity: arity(*op),
                })
                .collect(),
        );
    }

    RuleVocabulary {
        fields: field::known_fields(),
        operators,
    }
}

/// Compte les morceaux que des règles retiendraient, sans rien enregistrer.
///
/// Appelée pendant qu'on compose : voir le nombre bouger est ce qui permet de
/// comprendre une règle, bien plus qu'un intitulé.
#[tauri::command]
pub async fn count_smart_playlist(
    state: State<'_, AppState>,
    rules: rules::SmartRules,
) -> Result<i64, String> {
    engine::count(&state.pool, &rules).await
}

/// Rend un échantillon de ce que les règles donneraient.
#[tauri::command]
pub async fn preview_smart_playlist(
    state: State<'_, AppState>,
    rules: rules::SmartRules,
) -> Result<Vec<PlaylistTrackView>, String> {
    engine::evaluate(&state.pool, 0, &rules).await
}

/// Crée une playlist intelligente.
#[tauri::command]
pub async fn create_smart_playlist(
    state: State<'_, AppState>,
    profil_id: i64,
    name: String,
    color: Option<String>,
    icon: Option<String>,
    rules: rules::SmartRules,
    is_mix: Option<bool>,
) -> Result<i64, String> {
    // Les règles sont validées avant d'être enregistrées : une playlist qu'on
    // ne saurait pas évaluer ne doit pas exister.
    rules::build_where(&rules)?;
    rules::build_order(&rules.limit)?;

    let json = serde_json::to_string(&rules).map_err(|e| format!("Règles : {e}"))?;

    let (id,): (i64,) = sqlx::query_as(
        "INSERT INTO playlists (profil_id, name, color, icon, is_smart, rules, is_mix, pinned)
         VALUES (?, ?, ?, ?, 1, ?, ?, ?)
         RETURNING id",
    )
    .bind(profil_id)
    .bind(&name)
    .bind(color.unwrap_or_else(|| "#8b5cf6".to_string()))
    .bind(icon.unwrap_or_else(|| "lucide:sparkles".to_string()))
    .bind(&json)
    .bind(is_mix.unwrap_or(false))
    // Un mix ne s'épingle pas : il vit sur la page Mix.
    .bind(!is_mix.unwrap_or(false))
    .fetch_one(&state.pool)
    .await
    .map_err(|e| format!("Création : {e}"))?;

    refresh_count(&state.pool, id, &rules).await;

    Ok(id)
}

/// Réécrit les règles d'une playlist intelligente.
#[tauri::command]
pub async fn update_smart_playlist(
    state: State<'_, AppState>,
    playlist_id: i64,
    name: Option<String>,
    color: Option<String>,
    icon: Option<String>,
    rules: rules::SmartRules,
) -> Result<(), String> {
    rules::build_where(&rules)?;
    rules::build_order(&rules.limit)?;

    let json = serde_json::to_string(&rules).map_err(|e| format!("Règles : {e}"))?;

    sqlx::query(
        "UPDATE playlists SET
             name  = COALESCE(?, name),
             color = COALESCE(?, color),
             icon  = COALESCE(?, icon),
             rules = ?,
             is_smart = 1,
             updated_at = CURRENT_TIMESTAMP
         WHERE id = ?",
    )
    .bind(&name)
    .bind(&color)
    .bind(&icon)
    .bind(&json)
    .bind(playlist_id)
    .execute(&state.pool)
    .await
    .map_err(|e| format!("Mise à jour : {e}"))?;

    refresh_count(&state.pool, playlist_id, &rules).await;

    Ok(())
}

/// Lit les règles enregistrées d'une playlist.
#[tauri::command]
pub async fn get_smart_playlist_rules(
    state: State<'_, AppState>,
    playlist_id: i64,
) -> Result<Option<rules::SmartRules>, String> {
    let json: Option<String> =
        sqlx::query_scalar("SELECT rules FROM playlists WHERE id = ? AND is_smart = 1")
            .bind(playlist_id)
            .fetch_optional(&state.pool)
            .await
            .map_err(|e| format!("Lecture : {e}"))?
            .flatten();

    match json {
        Some(j) => serde_json::from_str(&j)
            .map(Some)
            .map_err(|e| format!("Règles illisibles : {e}")),
        None => Ok(None),
    }
}

/// Met à jour le compteur affiché dans la barre latérale.
///
/// Un échec n'interrompt rien : le compteur est un confort, et le perdre ne
/// justifie pas de refuser la création d'une playlist par ailleurs valide.
async fn refresh_count(pool: &sqlx::SqlitePool, id: i64, regles: &rules::SmartRules) {
    let Ok(n) = engine::count(pool, regles).await else {
        return;
    };
    let _ = sqlx::query("UPDATE playlists SET track_count = ? WHERE id = ?")
        .bind(n)
        .bind(id)
        .execute(pool)
        .await;
}

// ─── Auto-mixes ────────────────────────────────────────────────────────────

use crate::core::smart_playlist::auto_mix::{self, MixOptions};

/// Bilan d'une génération : ce qui est sorti du tirage, et l'espace dans
/// lequel il a puisé — deux chiffres que l'interface peut raconter.
#[derive(Debug, serde::Serialize)]
pub struct AutoMixResult {
    pub total: i64,
    pub pool: i64,
}

/// Tire une génération et la matérialise dans `playlist_items`. Partagé par
/// la création et le renouvellement : une génération est un geste unique,
/// pas deux chemins qui divergent.
async fn generer_auto_mix(
    pool_api: &sqlx::SqlitePool,
    playlist_id: i64,
    regles_json: &str,
    options_json: Option<&str>,
    recent: Option<&str>,
) -> Result<AutoMixResult, String> {
    let regles: rules::SmartRules =
        serde_json::from_str(regles_json).map_err(|e| format!("Règles : {e}"))?;
    let opts: MixOptions = options_json
        .and_then(|j| serde_json::from_str(j).ok())
        .unwrap_or_default();

    let candidats = auto_mix::candidats(pool_api, &regles).await?;
    let exclus = auto_mix::exclusions(recent);
    let graine = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_micros() as u64)
        .unwrap_or(1);
    let (choix, pool) = auto_mix::choisir(candidats, &opts, &exclus, graine);
    let ids: Vec<String> = choix.iter().map(|c| c.library_track_id.clone()).collect();

    let mut tx = pool_api.begin().await.map_err(|e| format!("transaction : {e}"))?;
    sqlx::query("DELETE FROM playlist_items WHERE playlist_id = ?")
        .bind(playlist_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| format!("purge : {e}"))?;
    for (rang, c) in choix.iter().enumerate() {
        sqlx::query("INSERT INTO playlist_items (playlist_id, library_track_id, sort_index) VALUES (?, ?, ?)")
            .bind(playlist_id)
            .bind(&c.library_track_id)
            .bind(rang as i64)
            .execute(&mut *tx)
            .await
            .map_err(|e| format!("insertion : {e}"))?;
    }
    let memoire = auto_mix::memoire_a_jour(recent, &ids);
    sqlx::query("UPDATE playlists SET mix_recent = ?, track_count = ?, duration = (SELECT COALESCE(SUM(lt.duration), 0) FROM playlist_items pi JOIN library_tracks lt ON lt.id = pi.library_track_id WHERE pi.playlist_id = playlists.id), updated_at = CURRENT_TIMESTAMP WHERE id = ?")
        .bind(&memoire)
        .bind(ids.len() as i64)
        .bind(playlist_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| format!("mémoire : {e}"))?;
    tx.commit().await.map_err(|e| format!("validation : {e}"))?;

    Ok(AutoMixResult { total: ids.len() as i64, pool: pool as i64 })
}

/// Crée un auto-mix et tire sa première génération : la playlist n'existe
/// jamais sans contenu.
#[tauri::command]
pub async fn create_auto_mix(
    state: State<'_, AppState>,
    profil_id: i64,
    name: String,
    color: Option<String>,
    icon: Option<String>,
    rules: rules::SmartRules,
    options: MixOptions,
) -> Result<i64, String> {
    rules::build_where(&rules)?;
    let regles_json = serde_json::to_string(&rules).map_err(|e| format!("Règles : {e}"))?;
    let options_json = serde_json::to_string(&options).map_err(|e| format!("Réglages : {e}"))?;

    let (id,): (i64,) = sqlx::query_as(
        "INSERT INTO playlists (profil_id, name, color, icon, is_auto, rules, mix_options, track_count, pinned)
         VALUES (?, ?, ?, ?, 1, ?, ?, 0, 1)
         RETURNING id",
    )
    .bind(profil_id)
    .bind(&name)
    .bind(color.unwrap_or_else(|| "#0ea5e9".to_string()))
    .bind(icon.unwrap_or_else(|| "lucide:dices"  .to_string()))
    .bind(&regles_json)
    .bind(&options_json)
    .fetch_one(&state.pool)
    .await
    .map_err(|e| format!("Création : {e}"))?;

    generer_auto_mix(&state.pool, id, &regles_json, Some(&options_json), None).await?;
    Ok(id)
}

/// Renouvelle la génération d'un auto-mix existant.
#[tauri::command]
pub async fn regenerate_auto_mix(
    state: State<'_, AppState>,
    playlist_id: i64,
) -> Result<AutoMixResult, String> {
    let ligne: Option<(Option<String>, Option<String>, Option<String>)> = sqlx::query_as(
        "SELECT rules, mix_options, mix_recent FROM playlists WHERE id = ? AND is_auto = 1",
    )
    .bind(playlist_id)
    .fetch_optional(&state.pool)
    .await
    .map_err(|e| format!("Lecture : {e}"))?;

    let (regles, options, recent) = ligne.ok_or("Auto-mix introuvable")?;
    let regles = regles.ok_or("Règles absentes")?;
    generer_auto_mix(&state.pool, playlist_id, &regles, options.as_deref(), recent.as_deref()).await
}
