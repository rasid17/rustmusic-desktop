/**
 * Shuffle « juste » : tirage reproductible (seed) puis répartition par artiste.
 *
 * Fisher-Yates pur est équitable mais pas lisible : il laisse un artiste
 * passer trois fois de suite, et le même ordre revient à chaque réouverture
 * de la même file. Le seed rend le tirage rejouable ; la passe de répartition
 * écarte les répétitions d'artiste sans jamais bloquer le mélange.
 */

import type { QueueTrack } from "#lib/types/db/queue/QueueTrack";

/** Un titre d'artiste absent n'est jamais en compétition avec lui-même. */
function cleArtiste(t: QueueTrack, index: number): string {
  return t.artist?.trim().toLowerCase() || `sans-artiste-${index}`;
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
 * Écarte les artistes : le même artiste revient au moins `gap` titres plus
 * loin quand c'est possible. À un artiste très majoritaire, la contrainte
 * devient impossible à tenir partout — on prend alors le premier disponible,
 * jamais de blocage.
 */
export function spreadByArtist(tracks: QueueTrack[], gap = 4): QueueTrack[] {
  const buckets = new Map<string, QueueTrack[]>();
  const cles = new Map<QueueTrack, string>();
  tracks.forEach((t, i) => {
    const cle = cleArtiste(t, i);
    cles.set(t, cle);
    const liste = buckets.get(cle);
    if (liste) liste.push(t);
    else buckets.set(cle, [t]);
  });

  const out: QueueTrack[] = [];
  const derniere = new Map<string, number>();
  while (out.length < tracks.length) {
    const recents = new Set(out.slice(-gap).map((t) => cles.get(t)!));
    let preferred: { cle: string; liste: QueueTrack[] } | null = null;
    let fallback: { cle: string; liste: QueueTrack[] } | null = null;
    let plusAncien = -1;
    for (const [cle, liste] of buckets) {
      if (liste.length === 0) continue;
      // Faute de choix « propre », le plus loin du dernier passage limite la
      // casse : deux mêmes artistes se suivent rarement, ils s'espacent.
      const age = out.length - (derniere.get(cle) ?? -tracks.length);
      if (age > plusAncien) { plusAncien = age; fallback = { cle, liste }; }
      if (recents.has(cle)) continue;
      if (!preferred || liste.length > preferred.liste.length) preferred = { cle, liste };
    }
    const prise = preferred ?? fallback;
    if (!prise) break;
    out.push(prise.liste.shift()!);
    derniere.set(prise.cle, out.length - 1);
    if (prise.liste.length === 0) buckets.delete(prise.cle);
  }
  return out;
}

/** Mélange complet : tirage sur seed puis répartition. Déterministe à seed donné. */
export function fairShuffle(tracks: QueueTrack[], seed: number, gap = 4): QueueTrack[] {
  return spreadByArtist(seededShuffle(tracks, seed), gap);
}
