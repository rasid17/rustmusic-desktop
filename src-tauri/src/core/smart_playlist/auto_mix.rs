//! Auto-mix : une playlist intelligente qui matérialise son résultat et ne
//! se répète pas.
//!
//! Trois choses le distinguent d'un mix ordinaire : un plafond par artiste
//! (le hasard pur laisse passer trois mêmes voix d'affilée), un équilibre
//! réglable entre favoris et redécouverte, et la mémoire des trois dernières
//! générations — le tirage d'après les exclut, c'est ce qui rend le
//! renouvellement visible plutôt que supposé.

use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

use super::field::TRACK_FROM;
use super::rules::{build_where, Bind, SmartRules};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MixOptions {
    /// Nombre de morceaux visés.
    #[serde(default = "def_longueur")]
    pub longueur: u32,
    /// Morceaux maximum par artiste dans le mix.
    #[serde(default = "def_par_artiste")]
    pub par_artiste: u32,
    /// 0 = les favoris d'abord, 100 = la redécouverte d'abord.
    #[serde(default = "def_fraicheur")]
    pub fraicheur: u32,
}

fn def_longueur() -> u32 { 50 }
fn def_par_artiste() -> u32 { 2 }
fn def_fraicheur() -> u32 { 50 }

impl Default for MixOptions {
    fn default() -> Self {
        Self { longueur: def_longueur(), par_artiste: def_par_artiste(), fraicheur: def_fraicheur() }
    }
}

/// Combien de générations passées le tirage nouveau exclut.
pub const GENERATIONS: usize = 3;

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct Candidat {
    pub library_track_id: String,
    pub artist_id: Option<String>,
    pub rating: Option<f64>,
    #[allow(dead_code)]
    pub play_count: i64,
    pub last_played_at: Option<DateTime<Utc>>,
}

/// Les pistes satisfying les règles, enrichies des colonnes de notation.
pub async fn candidats(pool: &SqlitePool, rules: &SmartRules) -> Result<Vec<Candidat>, String> {
    let filtre = build_where(rules)?;
    let sql = format!(
        "SELECT lt.id AS library_track_id, lt.artist_id, lt.rating, lt.play_count, lt.last_played_at\n\
         {TRACK_FROM}\nWHERE {}",
        filtre.sql
    );
    let mut q = sqlx::query_as::<_, Candidat>(sqlx::AssertSqlSafe(sql));
    for b in &filtre.binds {
        q = match b {
            Bind::Text(s) => q.bind(s.clone()),
            Bind::Number(n) => q.bind(*n),
        };
    }
    q.fetch_all(pool).await.map_err(|e| format!("Candidats auto-mix : {e}"))
}

/// mulberry32 — tirage léger et déterministe, de quoi casser la répétition
/// sans prétention cryptographique.
fn rng(seed: u64) -> impl FnMut() -> f64 {
    let mut a = seed as u32;
    move || {
        a = a.wrapping_add(0x6d2b79f5);
        let mut t = a;
        t = (t ^ (t >> 15)).wrapping_mul(t | 1);
        t ^= t.wrapping_add((t ^ (t >> 7)).wrapping_mul(t | 61));
        ((t ^ (t >> 14)) as u32) as f64 / u32::MAX as f64
    }
}

/// Favoris (0..1) tiré par la note, redécouverte (0..1) tirée par le temps
/// écoulé depuis la dernière écoute — quatre-vingt-dix jours sans toucher
/// une piste en font une presque neuve. Le réglage arbitre les deux, et un
/// tirage léger évite que deux générations se ressemblent trait pour trait.
fn score(c: &Candidat, opts: &MixOptions, tirage: f64) -> f64 {
    let favoris = match c.rating {
        Some(r) if r > 0.0 => (r / 5.0).min(1.0),
        _ => 0.2,
    };
    let fraicheur = match c.last_played_at {
        None => 1.0,
        Some(quand) => {
            let jours = (Utc::now() - quand).num_days().max(0) as f64;
            (jours / 90.0).min(1.0)
        }
    };
    let w = opts.fraicheur.clamp(0, 100) as f64 / 100.0;
    ((1.0 - w) * favoris + w * fraicheur) * 0.85 + tirage * 0.15
}

/// Sélectionne `longueur` pistes : par plafond d'artiste si possible, puis
/// complément par score si le plafond affame la liste. Rend aussi la taille
/// du pool, pour que l'interface dise d'où le mix est tiré.
pub fn choisir(
    candidats: Vec<Candidat>,
    opts: &MixOptions,
    exclus: &HashSet<String>,
    seed: u64,
) -> (Vec<Candidat>, usize) {
    let mut r = rng(seed);
    let mut classes: Vec<(f64, Candidat)> = candidats
        .into_iter()
        .filter(|c| !exclus.contains(&c.library_track_id))
        .map(|c| (score(&c, opts, r()), c))
        .collect();
    let pool = classes.len();
    classes.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));

    let plafond = opts.par_artiste.max(1);
    let cible = opts.longueur.max(1) as usize;
    let mut choisis: Vec<Candidat> = Vec::with_capacity(cible.min(pool));
    let mut par_artiste: HashMap<String, u32> = HashMap::new();

    for (_, c) in &classes {
        if choisis.len() >= cible {
            break;
        }
        let cle = c.artist_id.clone().unwrap_or_else(|| format!("#{}", c.library_track_id));
        if par_artiste.get(&cle).copied().unwrap_or(0) >= plafond {
            continue;
        }
        *par_artiste.entry(cle).or_insert(0) += 1;
        choisis.push(c.clone());
    }
    // Un artiste très majoritaire peut affamer la liste sous le plafond :
    // on complète avec les meilleurs restants plutôt que de servir un mix tronqué.
    if choisis.len() < cible {
        let pris: HashSet<String> = choisis.iter().map(|c| c.library_track_id.clone()).collect();
        for (_, c) in &classes {
            if choisis.len() >= cible {
                break;
            }
            if pris.contains(&c.library_track_id) {
                continue;
            }
            choisis.push(c.clone());
        }
    }
    (choisis, pool)
}

/// Lit les générations passées (`mix_recent`) en un ensemble d'exclusion et
/// la liste à jour après ajout d'une génération.
pub fn memoire_a_jour(recent: Option<&str>, nouvelle: &[String]) -> String {
    let mut generations: Vec<Vec<String>> = recent
        .and_then(|j| serde_json::from_str(j).ok())
        .unwrap_or_default();
    generations.push(nouvelle.to_vec());
    while generations.len() > GENERATIONS {
        generations.remove(0);
    }
    serde_json::to_string(&generations).unwrap_or_else(|_| "[]".to_string())
}

pub fn exclusions(recent: Option<&str>) -> HashSet<String> {
    let generations: Vec<Vec<String>> = recent
        .and_then(|j| serde_json::from_str(j).ok())
        .unwrap_or_default();
    generations.into_iter().flatten().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidat(id: &str, artiste: Option<&str>, rating: Option<f64>, jours: Option<i64>) -> Candidat {
        Candidat {
            library_track_id: id.to_string(),
            artist_id: artiste.map(str::to_string),
            rating,
            play_count: 0,
            last_played_at: jours.map(|j| Utc::now() - chrono::Duration::days(j)),
        }
    }

    fn options(longueur: u32, par_artiste: u32, fraicheur: u32) -> MixOptions {
        MixOptions { longueur, par_artiste, fraicheur }
    }

    #[test]
    fn plafond_artiste_tenu_quand_c_est_possible() {
        let mut pool = Vec::new();
        for i in 0..10 {
            pool.push(candidat(&format!("a{i}"), Some("Adele"), Some(5.0), None));
        }
        for i in 0..40 {
            pool.push(candidat(&format!("b{i}"), Some(&format!("Art{i}")), Some(3.0), None));
        }
        let (choix, _) = choisir(pool, &options(20, 2, 50), &HashSet::new(), 7);
        let mut par: HashMap<String, u32> = HashMap::new();
        for c in &choix {
            *par.entry(c.artist_id.clone().unwrap_or_default()).or_insert(0) += 1;
        }
        assert_eq!(choix.len(), 20);
        assert_eq!(par.get("Adele").copied().unwrap_or(0), 2, "Adele plafonnée à deux");
    }

    #[test]
    fn complete_quand_le_plafond_affame() {
        // Un seul artiste, 30 morceaux, longueur 20, plafond 2 : impossible
        // de tenir — la liste doit quand même faire 20.
        let pool: Vec<Candidat> =
            (0..30).map(|i| candidat(&format!("x{i}"), Some("Unique"), None, None)).collect();
        let (choix, _) = choisir(pool, &options(20, 2, 50), &HashSet::new(), 7);
        assert_eq!(choix.len(), 20);
    }

    #[test]
    fn exclut_les_generations_passees() {
        let pool: Vec<Candidat> =
            (0..10).map(|i| candidat(&format!("x{i}"), Some(&format!("A{i}")), None, None)).collect();
        let exclus: HashSet<String> = ["x0", "x1", "x2"].iter().map(|s| s.to_string()).collect();
        let (choix, pool_reste) = choisir(pool, &options(5, 2, 50), &exclus, 7);
        assert_eq!(pool_reste, 7);
        assert!(choix.iter().all(|c| !exclusions(Some(&serde_json::to_string(&vec![vec!["x0", "x1", "x2"]]).unwrap())).contains(&c.library_track_id)));
    }

    #[test]
    fn fraicheur_avantage_les_oubliees() {
        let pool = vec![
            candidat("notee", Some("A"), Some(5.0), Some(1)),
            candidat("oubliee", Some("B"), None, None),
        ];
        let opts = options(2, 2, 100);
        let (choix, _) = choisir(pool, &opts, &HashSet::new(), 1);
        assert_eq!(choix[0].library_track_id, "oubliee", "fraicheur 100 : l'oubliée passe avant");
    }

    #[test]
    fn memoire_garde_trois_generations() {
        let mut recent = String::new();
        for g in 0..5 {
            let gen: Vec<String> = vec![format!("g{g}1"), format!("g{g}2")];
            recent = memoire_a_jour(if recent.is_empty() { None } else { Some(&recent) }, &gen);
        }
        let generations: Vec<Vec<String>> = serde_json::from_str(&recent).unwrap();
        assert_eq!(generations.len(), GENERATIONS);
        assert_eq!(generations[0][0], "g21", "les deux premières générations sont parties");
    }
}
