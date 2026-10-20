/**
 * Pont entre la fenêtre principale et la fenêtre du mini-lecteur.
 *
 * La fenêtre principale reste le seul pilote de la lecture : file d'attente,
 * enchaînement sans blanc, historique, notifications et contrôles média OS.
 * La fenêtre mini est une télécommande affichée : elle reçoit l'état
 * (lecture, file, favoris), renvoie les actions de l'utilisateur, et anime
 * elle-même sa barre de progression — la fenêtre principale masquée ne peint
 * plus, elle n'envoie que des repères à la seconde.
 */

import { get } from "svelte/store";
import { listen, emitTo, type UnlistenFn } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { player } from "#lib/stores/player/player.store";
import type { Player } from "#lib/types/db/player/Player";
import type { AudioFile } from "#lib/types/db/audioFile/AudioFile";
import type { QueueTrack } from "#lib/types/db/queue/QueueTrack";
import { queueState } from "#lib/stores/queue/queueState.store";
import { liked } from "#lib/stores/playlist/like.store";
import { playerService } from "#lib/services/player/player.service";

/** Vrai dans la fenêtre « mini » (lecteur compact), faux dans la principale. */
export function isMiniWindow(): boolean {
  try {
    return getCurrentWindow().label === "mini";
  } catch {
    return false;
  }
}

/** Action de l'utilisateur, envoyée de la fenêtre mini à la principale. */
export function sendCmd(cmd: MiniCmd, payload?: MiniCmdPayload): void {
  emitTo("main", "mini-cmd", { cmd, payload }).catch((e) =>
    console.error("[mini-bridge] commande:", e)
  );
}

export type MiniCmd =
  | "toggle-play"
  | "next"
  | "prev"
  | "seek"
  | "play-at"
  | "reorder"
  | "remove-track"
  | "clear-upcoming"
  | "toggle-like"
  | "exit-mini"
  | "close-app";

export type MiniCmdPayload = {
  position?: number;
  index?: number;
  from?: number;
  to?: number;
  queueId?: string;
  path?: string;
};

type MiniCmdMessage = { cmd: MiniCmd; payload?: MiniCmdPayload };

/** Instantané du lecteur ; `audioFile` n'est présent qu'au changement de piste. */
type PlayerSnapshot = Omit<Player, "audioFile"> & { audioFile?: AudioFile | null };

// ─── Côté fenêtre principale ────────────────────────────────────────────────

export type MiniBridgeHandlers = {
  /** La fenêtre mini a été détruite (compositeur, Alt+F4, plateau) : tout restaurer. */
  onMiniClosed: () => void;
  /** Bouton « restaurer » de la fenêtre mini. */
  onExitMini: () => void;
  /** Bouton « fermer » de la fenêtre mini : plateau ou quitter, selon le réglage. */
  onCloseApp: () => void;
};

let actif = false;
let unlistens: UnlistenFn[] = [];
let unsubscribes: (() => void)[] = [];
let queueDebounce: ReturnType<typeof setTimeout> | null = null;
let readyResolvers: ((v: boolean) => void)[] = [];
let dernierePiste: string | null | undefined;
let lastTracksRef: QueueTrack[] | null = null;
let prevLiked: Set<string> = new Set();

const noop = () => {};

export function startMiniBridge(handlers: MiniBridgeHandlers): void {
  if (actif) return;
  actif = true;
  // Premier envoi complet : la piste « change » depuis rien.
  dernierePiste = undefined;
  lastTracksRef = null;
  prevLiked = new Set(get(liked).paths);

  unsubscribes.push(
    player.subscribe((p) => {
      const complet = p.pathFile !== dernierePiste;
      dernierePiste = p.pathFile;
      const snap: PlayerSnapshot = {
        status: p.status,
        pathFile: p.pathFile,
        duration: p.duration,
        rustPosition: p.rustPosition,
        jsPosition: p.jsPosition,
        isPreparing: p.isPreparing,
        trackId: p.trackId,
        lastUpdateTime: p.lastUpdateTime,
        ...(complet ? { audioFile: p.audioFile } : {}),
      };
      emitTo("mini", "mini-state-player", snap).catch(noop);
    })
  );

  unsubscribes.push(
    queueState.subscribe((qs) => {
      if (qs.tracks !== lastTracksRef) {
        lastTracksRef = qs.tracks;
        if (queueDebounce) clearTimeout(queueDebounce);
        queueDebounce = setTimeout(() => {
          queueDebounce = null;
          const s = get(queueState);
          emitTo("mini", "mini-state-queue", { tracks: s.tracks, currentIndex: s.currentIndex }).catch(noop);
        }, 200);
      } else {
        emitTo("mini", "mini-state-index", { currentIndex: qs.currentIndex }).catch(noop);
      }
    })
  );

  unsubscribes.push(
    liked.subscribe((s) => {
      const changes: { path: string; liked: boolean }[] = [];
      for (const p of s.paths) if (!prevLiked.has(p)) changes.push({ path: p, liked: true });
      for (const p of prevLiked) if (!s.paths.has(p)) changes.push({ path: p, liked: false });
      prevLiked = new Set(s.paths);
      if (changes.length > 0) emitTo("mini", "mini-state-liked", { changes }).catch(noop);
    })
  );

  void (async () => {
    // La fenêtre mini se présente : renvoyer l'état complet.
    unlistens.push(
      await listen("mini-hello", () => {
        const p = get(player);
        dernierePiste = p.pathFile;
        emitTo("mini", "mini-state-player", { ...p } as PlayerSnapshot).catch(noop);
        const qs = get(queueState);
        lastTracksRef = qs.tracks;
        emitTo("mini", "mini-state-queue", { tracks: qs.tracks, currentIndex: qs.currentIndex }).catch(noop);
        const paths = [...get(liked).paths].map((path) => ({ path, liked: true }));
        prevLiked = new Set(get(liked).paths);
        emitTo("mini", "mini-state-liked", { changes: paths }).catch(noop);
      })
    );
    unlistens.push(await listen("mini-ready", () => readyResolvers.splice(0).forEach((r) => r(true))));
    unlistens.push(await listen("mini-window-closed", () => handlers.onMiniClosed()));
    unlistens.push(
      await listen<MiniCmdMessage>("mini-cmd", (e) => {
        void dispatch(e.payload, handlers);
      })
    );
  })();
}

export function stopMiniBridge(): void {
  if (!actif) return;
  actif = false;
  unlistens.forEach((f) => f());
  unlistens = [];
  unsubscribes.forEach((f) => f());
  unsubscribes = [];
  if (queueDebounce) clearTimeout(queueDebounce);
  queueDebounce = null;
  readyResolvers = [];
}

/** Attend que la fenêtre mini se soit affichée ; `false` à l'expiration du délai. */
export function waitMiniReady(timeoutMs = 5000): Promise<boolean> {
  return new Promise((resolve) => {
    let done = false;
    const finish = (v: boolean) => {
      if (!done) {
        done = true;
        resolve(v);
      }
    };
    readyResolvers.push(finish);
    setTimeout(() => finish(false), timeoutMs);
  });
}

async function dispatch(msg: MiniCmdMessage, handlers: MiniBridgeHandlers): Promise<void> {
  const { cmd, payload } = msg ?? {};
  try {
    switch (cmd) {
      case "toggle-play":
        await playerService.handleTogglePlay();
        break;
      case "next":
        await playerService.nextTrack();
        break;
      case "prev":
        await playerService.prevTrack();
        break;
      case "seek":
        await playerService.seekTo(Number(payload?.position) || 0);
        break;
      case "play-at": {
        const index = Number(payload?.index);
        const track = get(queueState).tracks[index];
        if (!track) return;
        // La mutation de la file décide de la suite : sans ce drapeau, le
        // synchroniseur relancerait la lecture en concurrence avec playFile.
        playerService.expectExplicitAction();
        await queueState.setCurrentIndex(index);
        await playerService.playFile(track);
        break;
      }
      case "reorder":
        await queueState.reorderTracks(Number(payload?.from), Number(payload?.to));
        break;
      case "remove-track":
        if (payload?.queueId) await queueState.removeTrack(payload.queueId);
        break;
      case "clear-upcoming":
        await queueState.clearUpcoming();
        break;
      case "toggle-like":
        if (payload?.path) await liked.toggle(payload.path);
        break;
      case "exit-mini":
        handlers.onExitMini();
        break;
      case "close-app":
        handlers.onCloseApp();
        break;
      default:
        console.warn("[mini-bridge] commande inconnue:", cmd);
    }
  } catch (e) {
    console.error("[mini-bridge] commande échouée:", cmd, e);
  }
}

// ─── Côté fenêtre mini ──────────────────────────────────────────────────────

export type MiniClient = {
  /** Résolu au premier état reçu (ou après 800 ms) : la fenêtre peut s'afficher. */
  firstState: Promise<void>;
  dispose: () => void;
};

export function notifyMainReady(): void {
  emitTo("main", "mini-ready").catch(noop);
}

export function initMiniClient(): MiniClient {
  const unlistenFns: UnlistenFn[] = [];
  let firstResolve: () => void = () => {};
  const firstState = new Promise<void>((r) => (firstResolve = r));
  let recu = false;
  const premier = () => {
    if (recu) return;
    recu = true;
    firstResolve();
  };
  const filet = setTimeout(premier, 800);

  // Animation locale de la position : la fenêtre principale masquée n'envoie
  // que des repères à la seconde ; la fluidité est recalculée ici, avec la
  // même règle d'affichage (seconde entière ou dixième de seconde écoulé).
  let raf = 0;
  let basePos = 0;
  let baseTime = 0;
  const stopAnimation = () => {
    if (raf) cancelAnimationFrame(raf);
    raf = 0;
  };
  const startAnimation = () => {
    stopAnimation();
    let lastPush = 0;
    let lastWhole = -1;
    const loop = () => {
      const p = get(player);
      if (p.status !== "playing") {
        raf = 0;
        return;
      }
      if (!p.isPreparing) {
        const pos = basePos + (Date.now() - baseTime) / 1000;
        const now = Date.now();
        const whole = Math.floor(pos);
        if (whole !== lastWhole || now - lastPush >= 100) {
          lastWhole = whole;
          lastPush = now;
          player.update({ jsPosition: Math.min(pos, p.duration) });
        }
      }
      raf = requestAnimationFrame(loop);
    };
    raf = requestAnimationFrame(loop);
  };

  const appliquerSnapshot = (snap: PlayerSnapshot) => {
    const courant = get(player);
    const nouvellePiste = snap.pathFile !== courant.pathFile;
    // La position animée reste locale : celle de la fenêtre principale est
    // figée (fenêtre masquée), seul son repère Rust fait foi.
    const jsLocal =
      snap.status === "playing" && !snap.isPreparing && !nouvellePiste
        ? courant.jsPosition
        : snap.rustPosition;
    if (snap.audioFile !== undefined) player.setPlayer({ ...(snap as Player), jsPosition: jsLocal });
    else player.update({ ...snap, jsPosition: jsLocal });
    basePos = snap.rustPosition;
    baseTime = Date.now();
    if (snap.status === "playing" && !snap.isPreparing && !raf) startAnimation();
  };

  const miroirLiked = new Set<string>();

  void (async () => {
    unlistenFns.push(
      await listen<PlayerSnapshot>("mini-state-player", (e) => {
        appliquerSnapshot(e.payload);
        premier();
      })
    );
    unlistenFns.push(
      await listen<{ tracks: QueueTrack[]; currentIndex: number }>("mini-state-queue", (e) => {
        queueState.hydrate(e.payload);
        premier();
      })
    );
    unlistenFns.push(
      await listen<{ currentIndex: number }>("mini-state-index", (e) => {
        queueState.hydrate(e.payload);
      })
    );
    unlistenFns.push(
      await listen<{ changes: { path: string; liked: boolean }[] }>("mini-state-liked", (e) => {
        for (const c of e.payload.changes) {
          if (c.liked) miroirLiked.add(c.path);
          else miroirLiked.delete(c.path);
        }
        liked.hydrate(new Set(miroirLiked));
      })
    );
    emitTo("main", "mini-hello").catch(noop);
  })();

  return {
    firstState,
    dispose: () => {
      clearTimeout(filet);
      stopAnimation();
      unlistenFns.forEach((f) => f());
    },
  };
}
