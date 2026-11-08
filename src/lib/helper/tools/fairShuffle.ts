/**
 * Shuffle « juste » et intelligent : tirage reproductible (seed), espacement
 * proportionnel des artistes/albums sans biais de taille de collection, et
 * affinité de genre par rapport au titre d'ancrage en cours de lecture.
 */

import type { QueueTrack } from "#lib/types/db/queue/QueueTrack";

/** Un titre d'artiste absent n'est jamais en compétition avec lui-même. */
function cleArtiste(t: QueueTrack, index: number): string {
  return t.artist?.trim().toLowerCase() || `sans-artiste-${index}`;
}

function cleAlbum(t: QueueTrack): string | null {
  const alb = t.album?.trim().toLowerCase();
  return alb ? alb : null;
}

/** Découpe un tag de genre (« Indie Pop / Synthpop » -> ['indie pop', 'synthpop', 'pop']). */
function tokensGenre(genre?: string): string[] {
  if (!genre) return [];
  const raw = genre
    .toLowerCase()
    .split(/[,/;•|&]+/)
    .map((s) => s.trim())
    .filter(Boolean);
  const tokens = new Set<string>(raw);
  for (const part of raw) {
    for (const word of part.split(/\s+/)) {
      if (word.length >= 3) tokens.add(word);
    }
  }
  return [...tokens];
}

const GENRES_INSTRUMENTAUX_OU_CLASSIQUES = new Set([
  "classical",
  "classique",
  "orchestra",
  "orchestral",
  "symphony",
  "symphonic",
  "baroque",
  "opera",
  "chamber",
  "soundtrack",
  "ost",
  "score",
  "ambient",
]);

function estGenreClassiqueOuOrchestral(tokens: string[]): boolean {
  return tokens.some((tk) => GENRES_INSTRUMENTAUX_OU_CLASSIQUES.has(tk));
}

function scoreAffiniteGenre(
  candidatTokens: string[],
  ancreTokens: string[],
  precedentTokens: string[],
  positionDansFile: number,
): number {
  if (candidatTokens.length === 0) return 0;
  let score = 0;

  if (ancreTokens.length > 0) {
    const partageAncre = candidatTokens.some((tk) => ancreTokens.includes(tk));
    const ancreClassique = estGenreClassiqueOuOrchestral(ancreTokens);
    const candidatClassique = estGenreClassiqueOuOrchestral(candidatTokens);

    // Poids plus fort sur le début de la session d'écoute (les 35 premiers titres)
    const facteurDebut = positionDansFile < 35 ? 1.0 : 0.45;
    if (partageAncre) {
      score += 42 * facteurDebut;
    } else if (ancreClassique !== candidatClassique) {
      // Évite qu'un morceau d'orchestre/classique ne casse brutalement une session Pop/Rock (et inversement)
      score -= 55 * facteurDebut;
    }
  }

  if (precedentTokens.length > 0) {
    const partagePrecedent = candidatTokens.some((tk) => precedentTokens.includes(tk));
    if (partagePrecedent) {
      score += 18;
    }
  }

  return score;
}

export function newShuffleSeed(): number {
  return (Date.now() ^ Math.floor(Math.random() * 0x7fffffff)) >>> 0;
}

/** mulberry32 — petit PRNG déterministe, largement suffisant pour mélanger. */
function rng(seed: number): () => number {
  let a = seed >>> 0;
  return () => {
    a += 0x6d2b79f5;
    let t = a;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

export function seededShuffle<T>(arr: T[], seed: number): T[] {
  const a = [...arr];
  const r = rng(seed);
  for (let i = a.length - 1; i > 0; i--) {
    const j = Math.floor(r() * (i + 1));
    [a[i], a[j]] = [a[j], a[i]];
  }
  return a;
}

/**
 * Répartit les pistes déjà mélangées via une fenêtre glissante multi-critères :
 * - Supprime l'ancien biais « plus gros bucket d'abord » (`liste.length > preferred.liste.length`)
 *   qui faisait revenir en boucle les artistes ayant beaucoup de morceaux (ex. orchestre/OST).
 * - Écarte les répétitions d'un même artiste (`gap` titres) et d'un même album.
 * - Favorise la continuité de genre avec `anchorTrack` (le titre lancé par l'utilisateur)
 *   et la piste précédente.
 */
export function spreadByArtist(
  tracks: QueueTrack[],
  gap = 4,
  anchorTrack?: QueueTrack,
): QueueTrack[] {
  if (tracks.length <= 1) return [...tracks];

  const pool = [...tracks];
  const clesArtiste = new Map<QueueTrack, string>();
  const clesAlbum = new Map<QueueTrack, string | null>();
  const clesGenre = new Map<QueueTrack, string[]>();

  pool.forEach((t, i) => {
    clesArtiste.set(t, cleArtiste(t, i));
    clesAlbum.set(t, cleAlbum(t));
    clesGenre.set(t, tokensGenre(t.genre));
  });

  const out: QueueTrack[] = [];
  const dernierPassageArtiste = new Map<string, number>();
  const dernierPassageAlbum = new Map<string, number>();
  const ancreTokens = tokensGenre(anchorTrack?.genre);
  const albumGap = Math.max(2, gap - 1);

  if (anchorTrack) {
    const artAncre = cleArtiste(anchorTrack, -1);
    dernierPassageArtiste.set(artAncre, -1);
    const albAncre = cleAlbum(anchorTrack);
    if (albAncre) dernierPassageAlbum.set(albAncre, -1);
  }

  const FENETRE = 32;

  while (pool.length > 0) {
    const pos = out.length;
    const limite = Math.min(pool.length, FENETRE);
    const precedent = pos > 0 ? out[pos - 1] : anchorTrack;
    const precedentTokens = precedent ? (clesGenre.get(precedent) ?? tokensGenre(precedent.genre)) : [];

    let meilleurIdx = 0;
    let meilleurScore = -Infinity;

    for (let i = 0; i < limite; i++) {
      const candidat = pool[i];
      const art = clesArtiste.get(candidat)!;
      const alb = clesAlbum.get(candidat);
      const gTokens = clesGenre.get(candidat) ?? [];

      // Légère préférence pour l'ordre issu de seededShuffle afin de préserver l'équité du tirage
      let score = -i * 1.25;

      // 1. Pénalité de proximité d'artiste
      const dernierArt = dernierPassageArtiste.get(art);
      if (dernierArt !== undefined) {
        const distanceArt = pos - dernierArt;
        if (distanceArt <= gap) {
          score -= 1200 / distanceArt;
        }
      }

      // 2. Pénalité de proximité d'album (utile aussi sur les compilations / Various Artists)
      if (alb) {
        const dernierAlb = dernierPassageAlbum.get(alb);
        if (dernierAlb !== undefined) {
          const distanceAlb = pos - dernierAlb;
          if (distanceAlb <= albumGap) {
            score -= 350 / distanceAlb;
          }
        }
      }

      // 3. Affinité de genre avec le morceau d'ancrage et le morceau précédent
      score += scoreAffiniteGenre(gTokens, ancreTokens, precedentTokens, pos);

      if (score > meilleurScore) {
        meilleurScore = score;
        meilleurIdx = i;
      }
    }

    const [choisi] = pool.splice(meilleurIdx, 1);
    out.push(choisi);
    dernierPassageArtiste.set(clesArtiste.get(choisi)!, pos);
    const albChoisi = clesAlbum.get(choisi);
    if (albChoisi) {
      dernierPassageAlbum.set(albChoisi, pos);
    }
  }

  return out;
}

/** Mélange complet : tirage sur seed puis répartition intelligente. Déterministe à seed donné. */
export function fairShuffle(
  tracks: QueueTrack[],
  seed: number,
  gap = 4,
  anchorTrack?: QueueTrack,
): QueueTrack[] {
  return spreadByArtist(seededShuffle(tracks, seed), gap, anchorTrack);
}
