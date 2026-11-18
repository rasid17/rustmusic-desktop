/**
 * Géométrie et style de la fenêtre du mini-lecteur :
 * - 3 styles principaux : `classic` (Card / Bar / Large), `pill` (pochette ronde tournante), `sleeve` (CD réaliste tournant + égaliseur).
 * - Sous-mode `micro` (la pochette seule dans un carré redimensionnable).
 * - Tiroir inférieur déroulant (file d'attente / paroles) animé en hauteur.
 *
 * Ne s'exécute que dans la fenêtre « mini » — tout agit sur la fenêtre
 * courante. L'orchestration avec la fenêtre principale (ouvrir, masquer,
 * restaurer) vit dans `miniPlayer.store.ts`.
 */

import { writable, get } from "svelte/store";
import { availableMonitors, currentMonitor, getCurrentWindow, LogicalSize, PhysicalPosition } from "@tauri-apps/api/window";
import { invoke } from "@tauri-apps/api/core";
import { detectOS } from "#lib/helper/tools/osDetection";
import { loadPlatformFeatures, platformFeatures } from "#lib/stores/settings/platform.store";

export type MiniPlayerStyle = "classic" | "pill" | "sleeve";
export type MiniClassicLayout = "card" | "bar" | "large";

const MINI_STYLE_KEY = "rustmusic:mini-style";
const MINI_CLASSIC_LAYOUT_KEY = "rustmusic:mini-classic-layout";

function readMiniStyle(): MiniPlayerStyle {
  try {
    const v = localStorage.getItem(MINI_STYLE_KEY);
    if (v === "classic" || v === "pill" || v === "sleeve") return v;
  } catch {}
  return "pill";
}

function readMiniClassicLayout(): MiniClassicLayout {
  try {
    const v = localStorage.getItem(MINI_CLASSIC_LAYOUT_KEY);
    if (v === "card" || v === "bar" || v === "large") return v;
  } catch {}
  return "card";
}

/** Style actif du mini-lecteur : `classic` | `pill` | `sleeve`. */
export const miniPlayerStyle = writable<MiniPlayerStyle>(readMiniStyle());

/** Variante de mise en page lorsque le style `classic` est actif : `card` | `bar` | `large`. */
export const miniClassicLayout = writable<MiniClassicLayout>(readMiniClassicLayout());

function isMiniWindowContext(): boolean {
  try {
    return getCurrentWindow().label === "mini";
  } catch {
    return false;
  }
}

if (typeof window !== "undefined") {
  window.addEventListener("storage", (e) => {
    if (
      e.key === MINI_STYLE_KEY &&
      (e.newValue === "classic" || e.newValue === "pill" || e.newValue === "sleeve")
    ) {
      if (isMiniWindowContext()) void setMiniStyle(e.newValue);
      else miniPlayerStyle.set(e.newValue);
    } else if (
      e.key === MINI_CLASSIC_LAYOUT_KEY &&
      (e.newValue === "card" || e.newValue === "bar" || e.newValue === "large")
    ) {
      if (isMiniWindowContext()) void setMiniClassicLayout(e.newValue);
      else {
        miniClassicLayout.set(e.newValue);
        miniPlayerStyle.set("classic");
      }
    }
  });
}

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

/** Largeur logique selon le style et la variante active. */
export function currentMiniWidth(): number {
  const style = get(miniPlayerStyle);
  if (style === "pill") return 360;
  if (style === "sleeve") return 340;
  const layout = get(miniClassicLayout);
  if (layout === "bar") return 420;
  if (layout === "large") return 340;
  return 352;
}

/** Hauteur de repli initiale avant la mesure ResizeObserver. */
function defaultCollapsedHeight(style: MiniPlayerStyle, layout: MiniClassicLayout): number {
  if (style === "pill") return 136;
  if (style === "sleeve") return 372;
  if (layout === "bar") return 158;
  if (layout === "large") return 430;
  return 248;
}

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
const PANEL_H = 280;

/** Hauteur logique courante (suivie pour animer depuis la bonne valeur). */
let currentH = defaultCollapsedHeight(get(miniPlayerStyle), get(miniClassicLayout));
/** Hauteur réelle du contenu replié, mesurée par le composant (ResizeObserver). */
let collapsedH = currentH;
/** Panneau déroulé ou non (pour réajuster si le contenu change de taille). */
let expanded = false;
let resizeRaf = 0;
// Une bascule mini↔micro à la fois.
let bascule = false;

// GTK ignore `setSize` sur une fenêtre non redimensionnable : sous Linux, la taille
// passe par la taille demandée au widget (commande `set_compact_window_size`).
const gtkSize = detectOS() === "linux";

async function isWaylandSession(): Promise<boolean> {
  if (!gtkSize) return false;
  const cached = get(platformFeatures);
  if (cached.wayland) return true;
  return (await loadPlatformFeatures()).wayland;
}

async function applyMiniSize(width: number, height: number): Promise<void> {
  if (gtkSize) await invoke("set_compact_window_size", { width: Math.round(width), height: Math.round(height) });
  else await getCurrentWindow().setSize(new LogicalSize(width, height));
}

// Position du micro-lecteur, retenue d'une fois sur l'autre. Seul Wayland interdit
// la lecture/placement programmatique de position ; X11, Windows, etc. fonctionnent.
const MICRO_POS_KEY = "rustmusic:micro-pos";
let stopTrackingMicroPos: (() => void) | null = null;

async function restoreMicroPosition(): Promise<void> {
  if (await isWaylandSession()) return;
  try {
    const saved = JSON.parse(localStorage.getItem(MICRO_POS_KEY) ?? "null");
    if (typeof saved?.x !== "number" || typeof saved?.y !== "number") return;
    const visible = (await availableMonitors()).some(({ workArea: a }) =>
      saved.x >= a.position.x && saved.x < a.position.x + a.size.width &&
      saved.y >= a.position.y && saved.y < a.position.y + a.size.height);
    if (visible) await getCurrentWindow().setPosition(new PhysicalPosition(saved.x, saved.y));
  } catch (e) {
    console.error("[micro-player] position restore failed:", e);
  }
}

async function trackMicroPosition(): Promise<void> {
  if (await isWaylandSession()) return;
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

async function setWH(w: number, h: number): Promise<void> {
  currentH = h;
  try {
    await applyMiniSize(w, h);
  } catch (e) {
    console.error("[mini-player] setSize failed:", e);
  }
}

async function setH(h: number): Promise<void> {
  await setWH(currentMiniWidth(), h);
}

export function syncMiniStyleFromStorage(): void {
  const s = readMiniStyle();
  const l = readMiniClassicLayout();
  miniPlayerStyle.set(s);
  miniClassicLayout.set(l);
}

/** Change le style principal (`classic`, `pill`, `sleeve`) et redimensionne la fenêtre. */
export async function setMiniStyle(style: MiniPlayerStyle): Promise<void> {
  try {
    localStorage.setItem(MINI_STYLE_KEY, style);
  } catch {}
  miniPlayerStyle.set(style);
  if (get(microPlayerActive)) {
    await exitMicroPlayer();
    return;
  }
  const w = currentMiniWidth();
  const targetH = expanded ? collapsedH + PANEL_H : collapsedH;
  await setWH(w, targetH);
  await keepOnScreen(w, targetH);
}

/** Change la variante du mode `classic` (`card`, `bar`, `large`) et active `classic`. */
export async function setMiniClassicLayout(layout: MiniClassicLayout): Promise<void> {
  try {
    localStorage.setItem(MINI_CLASSIC_LAYOUT_KEY, layout);
    localStorage.setItem(MINI_STYLE_KEY, "classic");
  } catch {}
  miniClassicLayout.set(layout);
  miniPlayerStyle.set("classic");
  if (get(microPlayerActive)) {
    await exitMicroPlayer();
    return;
  }
  const w = currentMiniWidth();
  const targetH = expanded ? collapsedH + PANEL_H : collapsedH;
  await setWH(w, targetH);
  await keepOnScreen(w, targetH);
}

/** Cycle rapidement entre `classic` -> `pill` -> `sleeve`. */
export async function cycleMiniStyle(): Promise<void> {
  const cur = get(miniPlayerStyle);
  const next: MiniPlayerStyle = cur === "classic" ? "pill" : cur === "pill" ? "sleeve" : "classic";
  await setMiniStyle(next);
}

/** Garde la fenêtre entière dans la zone de travail de l'écran : collée au bord, elle ne déborde plus en grandissant. */
async function keepOnScreen(width: number, height: number): Promise<void> {
  if (await isWaylandSession()) return;
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

/** Redimensionne le micro en gardant le carré (poignée du coin). */
export async function resizeMicroPlayer(size: number): Promise<void> {
  if (!get(microPlayerActive)) return;
  microSize = Math.round(Math.max(MICRO_MIN, Math.min(MICRO_MAX, size)));
  if (gtkSize) return;
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
    const w = currentMiniWidth();
    currentH = collapsedH;
    await applyMiniSize(w, collapsedH);
    await keepOnScreen(w, collapsedH);
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
    const eased = 1 - Math.pow(1 - tt, 3);
    const h = Math.round(from + (to - from) * eased);
    void setH(h);
    if (tt < 1) resizeRaf = requestAnimationFrame(step);
  };
  step();
}

/** Ouvre (déroulé) ou ferme le panneau. */
export function setMiniExpanded(exp: boolean): void {
  expanded = exp;
  const w = currentMiniWidth();
  const to = exp ? collapsedH + PANEL_H : collapsedH;
  if (exp) void keepOnScreen(w, to);
  if (gtkSize) void setWH(w, to);
  else animateMiniHeight(to);
}

/**
 * Signale la hauteur réelle du contenu replié, mesurée par le composant.
 * La fenêtre s'ajuste au pixel près. Sans effet si le panneau est ouvert.
 */
export function reportCollapsedHeight(h: number): void {
  const nh = Math.round(h);
  if (nh <= 0) return;
  collapsedH = nh;
  const targetH = expanded ? collapsedH + PANEL_H : collapsedH;
  void setWH(currentMiniWidth(), targetH);
}
