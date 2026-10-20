import { writable } from "svelte/store";
import { invoke } from "@tauri-apps/api/core";
import { listen, emit, type UnlistenFn } from "@tauri-apps/api/event";

/** Volume du lecteur (0–100), partagé : glissière, raccourcis et pastille bit-perfect. */
export const volume = writable(80);

let minuteur: ReturnType<typeof setTimeout> | null = null;

// Fenêtres principale et mini : ce que l'une règle, l'autre le suit.
let syncUnlisten: UnlistenFn | null = null;

/** Écoute les changements de volume faits dans l'autre fenêtre. */
export function initVolumeSync(): void {
  if (syncUnlisten) return;
  void listen<number>("volume-sync", (e) => volume.set(e.payload)).then((f) => {
    syncUnlisten = f;
  });
}

/** Relit le volume du moteur (au démarrage, après une sourdine par raccourci). */
export async function loadVolume(): Promise<void> {
  try {
    const v = await invoke<number>("get_volume");
    volume.set(v);
    emit("volume-sync", v).catch(() => {});
  } catch (e) {
    console.error("[volume] lecture :", e);
  }
}

/** Tout de suite à l'écran, au plus toutes les 50 ms vers le moteur. */
export function applyVolume(v: number): void {
  const borne = Math.max(0, Math.min(100, Math.round(v)));
  volume.set(borne);
  if (minuteur) clearTimeout(minuteur);
  minuteur = setTimeout(() => {
    invoke("set_volume", { volume: borne }).catch((e) => console.error("[volume] réglage :", e));
    emit("volume-sync", borne).catch(() => {});
  }, 50);
}
