import { writable } from "svelte/store";
import { invoke } from "@tauri-apps/api/core";
import { listen, emit, type UnlistenFn } from "@tauri-apps/api/event";

/** Volume du lecteur (0–100), partagé : glissière, raccourcis et pastille bit-perfect. */
export const volume = writable(80);

let minuteur: ReturnType<typeof setTimeout> | null = null;

// Fenêtres principale et mini : ce que l'une règle, l'autre le suit.
let syncUnlisten: UnlistenFn | null = null;
let syncPromise: Promise<UnlistenFn> | null = null;
let syncGen = 0;

/** Écoute les changements de volume faits dans l'autre fenêtre. */
export function initVolumeSync(): void {
  if (syncUnlisten || syncPromise) return;
  const gen = ++syncGen;
  syncPromise = listen<number>("volume-sync", (e) => volume.set(e.payload)).then((f) => {
    if (gen !== syncGen) {
      f();
      return f;
    }
    syncUnlisten = f;
    syncPromise = null;
    return f;
  }).catch((e) => {
    syncPromise = null;
    throw e;
  });
}

/** Libère l'écouteur de volume (notamment à la fermeture du mini-lecteur). */
export function disposeVolumeSync(): void {
  syncGen++;
  syncPromise = null;
  if (syncUnlisten) {
    syncUnlisten();
    syncUnlisten = null;
  }
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
