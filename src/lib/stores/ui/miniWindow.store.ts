/**
 * Géométrie de fenêtre du mini-lecteur : taille ajustée au contenu, panneau
 * déroulant (file d'attente / paroles) animé en hauteur, et micro-lecteur
 * (la pochette seule, dans un carré redimensionnable).
 *
 * Ne s'exécute que dans la fenêtre « mini » — tout agit sur la fenêtre
 * courante. L'orchestration avec la fenêtre principale (ouvrir, masquer,
 * restaurer) vit dans `miniPlayer.store.ts`.
 */

import { writable, get } from "svelte/store";
import { availableMonitors, currentMonitor, getCurrentWindow, LogicalSize, PhysicalPosition } from "@tauri-apps/api/window";
import { invoke } from "@tauri-apps/api/core";
import { detectOS } from "#lib/helper/tools/osDetection";

/** Micro-lecteur : sous-mode du mini, la pochette seule dans un carré. */
export const microPlayerActive = writable(false);

/** Épinglé au-dessus des autres fenêtres, retenu d'une fois sur l'autre. */
const MINI_PIN_KEY = "rustmusic:mini-pin";
export const miniPinned = writable(readMiniPinned());

export function readMiniPinned(): boolean {
  try {
    return localStorage.getItem(MINI_PIN_KEY) !== "false";
  } catch {
    return true;
  }
}

/** Garde ou non la fenêtre au-dessus des autres. */
export async function toggleMiniPin(): Promise<void> {
  const epingle = !get(miniPinned);
  try {
    await getCurrentWindow().setAlwaysOnTop(epingle);
    miniPinned.set(epingle);
    try {
      localStorage.setItem(MINI_PIN_KEY, String(epingle));
    } catch {}
  } catch (e) {
    console.error("[mini-player] setAlwaysOnTop failed:", e);
  }
}

/** Largeur fixe (points logiques) du mini-lecteur. */
const MINI_W = 380;
/** Hauteur de repli par défaut, remplacée par la mesure réelle du contenu. */
const MINI_H = 150;
/** Côté du micro-lecteur (points logiques) : de 220 à trois fois plus, retenu d'une fois sur l'autre. */
export const MICRO_MIN = 220;
export const MICRO_MAX = 660;
const MICRO_SIZE_KEY = "rustmusic:micro-size";
let microSize = (() => {
  try {
    const n = Number(localStorage.getItem(MICRO_SIZE_KEY));
    return n >= MICRO_MIN && n <= MICRO_MAX ? n : MICRO_MIN;
  } catch {
    return MICRO_MIN;
  }
})();
/** Hauteur ajoutée par le panneau (file d'attente / paroles) quand déroulé. */
const PANEL_H = 300;

/** Hauteur logique courante (suivie pour animer depuis la bonne valeur). */
let currentH = MINI_H;
/** Hauteur réelle du contenu replié, mesurée par le composant (ResizeObserver). */
let collapsedH = MINI_H;
/** Panneau déroulé ou non (pour réajuster si le contenu change de taille). */
let expanded = false;
let resizeRaf = 0;
// Une bascule mini↔micro à la fois.
let bascule = false;

// GTK ignore `setSize` sur une fenêtre non redimensionnable : sous Linux, la taille
// passe par la taille demandée au widget (commande `set_compact_window_size`).
// La fenêtre mini est créée redimensionnable puis figée par MiniRoot une fois
// affichée — le même enchaînement que la principale en entrant en mode mini.
const gtkSize = detectOS() === "linux";

async function applyMiniSize(width: number, height: number): Promise<void> {
  if (gtkSize) await invoke("set_compact_window_size", { width: Math.round(width), height: Math.round(height) });
  else await getCurrentWindow().setSize(new LogicalSize(width, height));
}

// Position du micro-lecteur, retenue d'une fois sur l'autre. Wayland ne donne ni ne
// laisse choisir la position d'une fenêtre : rien à retenir sous Linux.
const MICRO_POS_KEY = "rustmusic:micro-pos";
let stopTrackingMicroPos: (() => void) | null = null;

async function restoreMicroPosition(): Promise<void> {
  if (gtkSize) return;
  try {
    const saved = JSON.parse(localStorage.getItem(MICRO_POS_KEY) ?? "null");
    if (typeof saved?.x !== "number" || typeof saved?.y !== "number") return;
    // Écran débranché depuis : on laisse la fenêtre où elle est.
    const visible = (await availableMonitors()).some(({ workArea: a }) =>
      saved.x >= a.position.x && saved.x < a.position.x + a.size.width &&
      saved.y >= a.position.y && saved.y < a.position.y + a.size.height);
    if (visible) await getCurrentWindow().setPosition(new PhysicalPosition(saved.x, saved.y));
  } catch (e) {
    console.error("[micro-player] position restore failed:", e);
  }
}

async function trackMicroPosition(): Promise<void> {
  if (gtkSize) return;
  untrackMicroPosition();
  stopTrackingMicroPos = await getCurrentWindow().onMoved(({ payload }) => {
    try {
      localStorage.setItem(MICRO_POS_KEY, JSON.stringify({ x: payload.x, y: payload.y }));
    } catch {}
  });
}

function untrackMicroPosition(): void {
  stopTrackingMicroPos?.();
  stopTrackingMicroPos = null;
}

async function setH(h: number): Promise<void> {
  currentH = h;
  try {
    await applyMiniSize(MINI_W, h);
  } catch (e) {
    console.error("[mini-player] setSize failed:", e);
  }
}

/** Garde la fenêtre entière dans la zone de travail de l'écran : collée au bord, elle ne déborde plus en grandissant. */
async function keepOnScreen(width: number, height: number): Promise<void> {
  try {
    const win = getCurrentWindow();
    const monitor = await currentMonitor();
    if (!monitor) return;
    const scale = await win.scaleFactor();
    const pos = await win.outerPosition();
    const area = monitor.workArea;
    const maxX = area.position.x + area.size.width - Math.round(width * scale);
    const maxY = area.position.y + area.size.height - Math.round(height * scale);
    const x = Math.max(area.position.x, Math.min(pos.x, maxX));
    const y = Math.max(area.position.y, Math.min(pos.y, maxY));
    if (x !== pos.x || y !== pos.y) {
      // Wayland laisse le compositeur décider de la place : le « move » du
      // client n'est jamais acquitté. On tente une seconde, pas plus.
      await Promise.race([
        win.setPosition(new PhysicalPosition(x, y)),
        new Promise((r) => setTimeout(r, 1000)),
      ]);
    }
  } catch (e) {
    console.error("[mini-player] keepOnScreen failed:", e);
  }
}

/** Du mini au micro : la fenêtre devient un carré, le mini ne se remesure plus. */
export async function enterMicroPlayer(): Promise<void> {
  if (get(microPlayerActive) || bascule) return;
  bascule = true;
  cancelAnimationFrame(resizeRaf);
  try {
    untrackMicroPosition();
    await applyMiniSize(microSize, microSize);
    await restoreMicroPosition();
    await keepOnScreen(microSize, microSize);
    microPlayerActive.set(true);
    await trackMicroPosition();
  } catch (e) {
    console.error("[micro-player] enter failed:", e);
  } finally {
    bascule = false;
  }
}

/** Redimensionne le micro en gardant le carré (poignée du coin), sans recaler pendant le geste. */
export async function resizeMicroPlayer(size: number): Promise<void> {
  if (!get(microPlayerActive)) return;
  const next = Math.round(Math.max(MICRO_MIN, Math.min(MICRO_MAX, size)));
  // Sous Wayland, rétrécir re-mappe la fenêtre : pendant le glisser, on ne
  // laisse passer que les agrandissements ; le repli se fait d'un coup à la fin.
  if (gtkSize && next < microSize) {
    microSize = next;
    return;
  }
  microSize = next;
  try {
    await applyMiniSize(microSize, microSize);
  } catch (e) {
    console.error("[micro-player] resize failed:", e);
  }
}

/** Fin du geste : la taille définitive est appliquée, la fenêtre revient dans l'écran. */
export async function endMicroResize(): Promise<void> {
  try {
    await applyMiniSize(microSize, microSize);
  } catch (e) {
    console.error("[micro-player] resize failed:", e);
  }
  await keepOnScreen(microSize, microSize);
  try {
    localStorage.setItem(MICRO_SIZE_KEY, String(microSize));
  } catch {}
}

export function currentMicroSize(): number {
  return microSize;
}

/** Retour au mini, panneau replié. */
export async function exitMicroPlayer(): Promise<void> {
  if (!get(microPlayerActive) || bascule) return;
  bascule = true;
  cancelAnimationFrame(resizeRaf);
  untrackMicroPosition();
  try {
    expanded = false;
    currentH = collapsedH;
    await applyMiniSize(MINI_W, collapsedH);
    await keepOnScreen(MINI_W, collapsedH);
    microPlayerActive.set(false);
  } catch (e) {
    console.error("[micro-player] exit failed:", e);
  } finally {
    bascule = false;
  }
}

/**
 * Anime la hauteur de fenêtre de la valeur courante vers `to`
 * (easeOutCubic, ~220 ms). Donne l'effet de déroulement fluide du panneau.
 */
function animateMiniHeight(to: number): void {
  cancelAnimationFrame(resizeRaf);
  const from = currentH;
  if (from === to) return;
  const start = Date.now();
  const dur = 220;
  const step = () => {
    const tt = Math.min(1, (Date.now() - start) / dur);
    const eased = 1 - Math.pow(1 - tt, 3); // easeOutCubic
    const h = Math.round(from + (to - from) * eased);
    void setH(h);
    if (tt < 1) resizeRaf = requestAnimationFrame(step);
  };
  step();
}

/** Ouvre (déroulé) ou ferme le panneau, avec animation. */
export function setMiniExpanded(exp: boolean): void {
  expanded = exp;
  const to = exp ? collapsedH + PANEL_H : collapsedH;
  // Sous Wayland la fenêtre ne se déplace pas elle-même : le recadrage reste
  // tenté (utile en X11) mais l'animation ne l'attend pas.
  if (exp) void keepOnScreen(MINI_W, to);
  // Wayland ne rétrécit pas une fenêtre image par image : le repli d'un seul coup.
  if (!exp && gtkSize) void setH(collapsedH);
  else animateMiniHeight(to);
}

/**
 * Signale la hauteur réelle du contenu replié (top bar + lecteur + progression
 * + onglets), mesurée par le composant. La fenêtre s'ajuste au pixel près :
 * ni bande vide en bas, ni onglets coupés. Sans effet si le panneau est ouvert.
 */
export function reportCollapsedHeight(h: number): void {
  const nh = Math.round(h);
  if (nh <= 0 || Math.abs(nh - collapsedH) < 1) return;
  collapsedH = nh;
  if (!expanded) void setH(collapsedH);
}
