/**
 * Mode « mini-player » : ouvre le lecteur compact dans une fenêtre distincte
 * (image-dans-l'image pour les compositeurs Wayland, titre
 * « Picture-in-Picture » que niri fait flotter d'office) et masque la fenêtre
 * principale tant qu'elle est affichée.
 *
 * La fenêtre principale n'est jamais redimensionnée : elle garde sa taille,
 * ses règles et son comportement habituels. La géométrie de la fenêtre mini
 * vit dans `miniWindow.store.ts`, et les échanges entre les deux fenêtres
 * passent par `miniBridge` (la principale pilote, la mini affiche et commande).
 */

import { writable, get } from "svelte/store";
import { getCurrentWindow, LogicalSize } from "@tauri-apps/api/window";
import { invoke } from "@tauri-apps/api/core";
import { startMiniBridge, stopMiniBridge, waitMiniReady } from "#lib/services/mini/miniBridge";
import { readMiniPinned } from "#lib/stores/ui/miniWindow.store";
import { settingsStore } from "#lib/stores/settings/settings.store";

export const miniPlayerActive = writable(false);

/** Plancher du mode normal : en dessous, l'interface n'est plus utilisable. */
const NORMAL_MIN_W = 800;
const NORMAL_MIN_H = 560;
/** Taille de départ (tauri.conf.json), quand la taille gardée n'est pas fiable. */
const NORMAL_W = 1280;
const NORMAL_H = 900;

// Une bascule à la fois : un double clic ouvrirait deux fenêtres mini.
let bascule = false;

export async function enterMiniPlayer(): Promise<void> {
  if (get(miniPlayerActive) || bascule) return;
  bascule = true;
  try {
    startMiniBridge({
      onMiniClosed: () => void restoreMainWindow(),
      onExitMini: () => void exitMiniPlayer(),
      onCloseApp: () => void closeApplication(),
    });
    await invoke("open_mini_window", { pinned: readMiniPinned() });
    // La fenêtre principale ne se masque qu'une fois la mini affichée :
    // jamais de moment sans fenêtre visible.
    if (!(await waitMiniReady())) throw new Error("fenêtre mini non prête");
    miniPlayerActive.set(true);
    await getCurrentWindow().hide();
  } catch (e) {
    console.error("[mini-player] enter failed:", e);
    miniPlayerActive.set(false);
    stopMiniBridge();
    await invoke("close_mini_window").catch(() => {});
  } finally {
    bascule = false;
  }
}

export async function exitMiniPlayer(): Promise<void> {
  if (!get(miniPlayerActive) || bascule) return;
  bascule = true;
  try {
    miniPlayerActive.set(false);
    stopMiniBridge();
    await invoke("close_mini_window");
    await showMainWindow();
  } catch (e) {
    console.error("[mini-player] exit failed:", e);
  } finally {
    bascule = false;
  }
}

/** Fenêtre mini fermée de l'extérieur (compositeur, Alt+F4, plateau) : la principale revient. */
async function restoreMainWindow(): Promise<void> {
  if (!get(miniPlayerActive)) return;
  miniPlayerActive.set(false);
  stopMiniBridge();
  await showMainWindow();
}

async function showMainWindow(): Promise<void> {
  try {
    const win = getCurrentWindow();
    await win.show();
    await win.setFocus();
  } catch (e) {
    console.error("[mini-player] show main failed:", e);
  }
}

/** Bouton « fermer » de la fenêtre mini : plateau ou quitter, selon le réglage. */
async function closeApplication(): Promise<void> {
  // Tout est réinitialisé avant la destruction : l'événement `mini-window-closed`
  // qui suit ne doit plus rien déclencher.
  miniPlayerActive.set(false);
  stopMiniBridge();
  await invoke("close_mini_window").catch(() => {});
  const win = getCurrentWindow();
  if (settingsStore.get("minimize_to_tray") === "true") {
    await win.hide();
    return;
  }
  await win.close();
}

export async function toggleMiniPlayer(): Promise<void> {
  if (get(miniPlayerActive)) await exitMiniPlayer();
  else await enterMiniPlayer();
}

/**
 * Au démarrage : plancher du mode normal, et taille d'origine si une ancienne
 * version avait fermé l'appli en mode mini (la taille réduite avait alors pu
 * être gardée pour la prochaine fois).
 */
export async function ensureNormalSize(): Promise<void> {
  try {
    const win = getCurrentWindow();
    await win.setMinSize(new LogicalSize(NORMAL_MIN_W, NORMAL_MIN_H));
    const taille = (await win.innerSize()).toLogical(await win.scaleFactor());
    if (taille.width < NORMAL_MIN_W || taille.height < NORMAL_MIN_H) {
      await win.setSize(new LogicalSize(NORMAL_W, NORMAL_H));
      await win.center();
    }
  } catch (e) {
    console.error("[mini-player] normal size check failed:", e);
  }
}
