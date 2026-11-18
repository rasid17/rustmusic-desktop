import { get } from "svelte/store";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { player } from "#lib/stores/player/player.store";
import { queueState } from "#lib/stores/queue/queueState.store";
import { liked } from "#lib/stores/playlist/like.store";
import { volume, applyVolume, loadVolume } from "#lib/stores/player/volume.store";
import { playbackPipelineStore, pipelineMode } from "#lib/stores/player/playbackPipeline.store";
import { playerService } from "#lib/services/player/player.service";
import { handlePlayTrack } from "#lib/actions/player/PlayerAction";
import { toQueue, toQueueTrack } from "#lib/mapper/queue/mapQueueTrack";
import { libraryStore } from "#lib/stores/library/library.store";
import { playlistStore } from "#lib/stores/playlist/playlist.store";
import { profilSelector } from "#lib/stores/profil/profil.store";
import { sourceFilter } from "#lib/stores/source/sourceFilter.store";
import type { TrackListView } from "#lib/types/ui/library/track/TrackListView";
import type { RecentFileListView } from "#lib/types/ui/recent/RecentFileListView";

export type MobileRemoteStatus = {
  running: boolean;
  port: number;
  pin_enabled: boolean;
  pin: string;
  url: string | null;
  qr_svg: string | null;
  connected_clients: number;
};

type RemoteCmdPayload = {
  cmd: string;
  position?: number;
  volume?: number;
  index?: number;
  from?: number;
  to?: number;
  queue_id?: string;
  path?: string;
  paths?: string[];
  track_id?: string;
  album_id?: string;
  artist_id?: string;
  playlist_id?: string;
  shuffle?: boolean;
};

let initialized = false;
let unlistenCmd: UnlistenFn | null = null;
let unsubs: (() => void)[] = [];
let pushTimer: ReturnType<typeof setTimeout> | null = null;
let lastSig = "";

function schedulePush(immediate = false) {
  if (pushTimer) {
    if (!immediate) return;
    clearTimeout(pushTimer);
    pushTimer = null;
  }
  const delay = immediate ? 0 : 80;
  pushTimer = setTimeout(() => {
    pushTimer = null;
    void pushStateNow();
  }, delay);
}

async function pushStateNow() {
  const p = get(player);
  const qs = get(queueState);
  const vol = get(volume);
  const likedPaths = get(liked).paths;
  const pipe = get(playbackPipelineStore);

  const curTrack = qs.tracks[qs.currentIndex] ?? null;
  const title =
    p.audioFile?.tags?.title ??
    curTrack?.title ??
    (p.pathFile ? p.pathFile.split(/[\\/]/).pop() ?? "" : "");
  const artist = p.audioFile?.tags?.artist ?? curTrack?.artist ?? "";
  const album = p.audioFile?.tags?.album ?? curTrack?.album ?? "";
  const thumb = curTrack?.cover ?? p.audioFile?.tags?.attached_images?.[0]?.image_src ?? null;
  const isLiked = p.pathFile ? likedPaths.has(p.pathFile) : false;
  const mode = pipelineMode(pipe, vol);
  const dopActive = pipe?.backend?.endsWith("DoP") === true;

  const pipelineLabel =
    mode === "dop"
      ? "Native DSD (DoP)"
      : mode === "bit-perfect"
        ? "Bit-Perfect"
        : mode === "exclusive"
          ? "Exclusive"
          : mode === "dsd"
            ? "DSD"
            : null;

  const queueItems = qs.tracks.slice(0, 250).map((t, idx) => ({
    queue_id: t.queueId,
    index: idx,
    title: t.title ?? "Untitled",
    artist: t.artist ?? "",
    album: t.album ?? "",
    path: t.path,
    duration: t.duration ?? 0,
    thumbnail_path: t.cover ?? null,
  }));

  const payload = {
    status: p.status ?? "stopped",
    title,
    artist,
    album,
    path: p.pathFile ?? null,
    track_id: p.trackId ? String(p.trackId) : null,
    duration: p.duration ?? 0,
    position: p.rustPosition ?? 0,
    volume: Math.max(0, Math.min(100, Math.round(vol))),
    muted: vol === 0,
    dop_active: dopActive,
    is_shuffled: !!qs.isShuffled,
    repeat_mode: qs.repeatMode ?? "off",
    liked: isLiked,
    thumbnail_path: thumb,
    audio_format: pipe?.source_format ?? null,
    bits_per_sample: pipe?.source_bits ?? null,
    sample_rate: pipe?.source_sample_rate ?? null,
    bitrate: null,
    pipeline_label: pipelineLabel,
    current_index: qs.currentIndex ?? -1,
    queue: queueItems,
    updated_at_ms: 0,
  };

  try {
    await invoke("mobile_remote_push_state", { payload });
  } catch {
    // Ignore when server is stopped
  }
}

async function resolveTracksByPaths(paths: string[]): Promise<TrackListView[]> {
  if (!paths.length) return [];
  try {
    return await invoke<TrackListView[]>("get_tracks_view_by_paths", { paths });
  } catch {
    return [];
  }
}

async function handleRemoteCmd(payload: RemoteCmdPayload) {
  const { cmd } = payload;
  const defaultLibId =
    get(libraryStore).librarySelected?.id ??
    get(libraryStore).libraries[0]?.id ??
    1;

  try {
    switch (cmd) {
      case "toggle-play":
        await playerService.handleTogglePlay();
        schedulePush(true);
        break;
      case "next":
        await playerService.nextTrack();
        schedulePush(true);
        break;
      case "prev":
        await playerService.prevTrack();
        schedulePush(true);
        break;
      case "seek":
        await playerService.seekTo(Number(payload.position) || 0);
        schedulePush(true);
        break;
      case "set-volume":
        if (typeof payload.volume === "number") {
          applyVolume(Math.max(0, Math.min(100, payload.volume)));
          schedulePush(true);
        }
        break;
      case "toggle-mute":
        await invoke("mute");
        await loadVolume();
        schedulePush(true);
        break;
      case "toggle-shuffle":
        await queueState.toggleShuffle();
        schedulePush(true);
        break;
      case "reshuffle":
        await queueState.reshuffle();
        schedulePush(true);
        break;
      case "cycle-repeat": {
        const cur = get(queueState).repeatMode;
        const next = cur === "off" ? "all" : cur === "all" ? "one" : "off";
        await queueState.setRepeatMode(next);
        schedulePush(true);
        break;
      }
      case "toggle-like":
        if (payload.path) {
          await liked.toggle(payload.path);
          schedulePush(true);
        }
        break;
      case "play-at": {
        const idx = Number(payload.index);
        const track = get(queueState).tracks[idx];
        if (!track) return;
        playerService.expectExplicitAction();
        await queueState.setCurrentIndex(idx);
        await playerService.playFile(track);
        schedulePush(true);
        break;
      }
      case "remove-queue-item":
        if (payload.queue_id) {
          await queueState.removeTrack(payload.queue_id);
          schedulePush(true);
        }
        break;
      case "clear-upcoming":
        await queueState.clearUpcoming();
        schedulePush(true);
        break;
      case "play-track":
        if (payload.path) {
          const contextPaths =
            Array.isArray(payload.paths) && payload.paths.length > 1
              ? payload.paths
              : [];
          let views =
            contextPaths.length > 1
              ? await resolveTracksByPaths(contextPaths)
              : await resolveTracksByPaths([payload.path]);

          const existingQueueTrack = get(queueState).tracks.find((t) => t.path === payload.path);
          const targetInViews = views.some((v) => v.path === payload.path);
          if (!targetInViews && !existingQueueTrack) {
            console.warn("[mobile-remote] blocked unverified play-track path");
            return;
          }

          // If only 1 track was in the list (e.g. single search result), load a broader library queue context
          if (targetInViews && views.length <= 1) {
            try {
              const page = await invoke<{ tracks: TrackListView[] }>("get_tracks_paginated", {
                libraryId: defaultLibId,
                offset: 0,
                limit: 150,
                sortKey: "title",
                sortDir: "asc",
                filter: null,
                missingCover: false,
                quality: null,
                favorites: false,
                genre: null,
              });
              if (page?.tracks?.length > 1) {
                const hasSelected = page.tracks.some((t) => t.path === payload.path);
                views = hasSelected ? page.tracks : [...views, ...page.tracks];
              }
            } catch {
              // Keep views as-is if fallback fails
            }
          }

          if (views.length > 0 && targetInViews) {
            await handlePlayTrack(payload.path, toQueue(views));
          } else if (existingQueueTrack) {
            await handlePlayTrack(existingQueueTrack.path, get(queueState).tracks);
          }
          schedulePush(true);
        }
        break;
      case "play-next":
        if (payload.path) {
          const views = await resolveTracksByPaths([payload.path]);
          const existingQueueTrack = get(queueState).tracks.find((t) => t.path === payload.path);
          if (views.length > 0) {
            await queueState.addTrack(toQueueTrack(views[0]));
          } else if (existingQueueTrack) {
            await queueState.addTrack(existingQueueTrack);
          } else {
            console.warn("[mobile-remote] blocked unverified play-next path");
            return;
          }
          schedulePush(true);
        }
        break;
      case "enqueue-track":
        if (payload.path) {
          const views = await resolveTracksByPaths([payload.path]);
          const existingQueueTrack = get(queueState).tracks.find((t) => t.path === payload.path);
          if (views.length > 0) {
            await queueState.enqueue(toQueueTrack(views[0]));
          } else if (existingQueueTrack) {
            await queueState.enqueue(existingQueueTrack);
          } else {
            console.warn("[mobile-remote] blocked unverified enqueue-track path");
            return;
          }
          schedulePush(true);
        }
        break;
      case "play-album":
        if (payload.album_id) {
          const tracks = await invoke<TrackListView[]>("get_tracks_by_album", {
            libraryId: defaultLibId,
            albumId: payload.album_id,
          });
          if (tracks.length > 0 && tracks[0].path) {
            await handlePlayTrack(tracks[0].path, toQueue(tracks));
            schedulePush(true);
          }
        }
        break;
      case "play-artist":
        if (payload.artist_id) {
          const tracks = await invoke<TrackListView[]>("get_tracks_by_artist", {
            libraryId: defaultLibId,
            artistId: payload.artist_id,
          });
          if (tracks.length > 0 && tracks[0].path) {
            await handlePlayTrack(tracks[0].path, toQueue(tracks));
            schedulePush(true);
          }
        }
        break;
      case "play-playlist":
        if (payload.playlist_id) {
          let tracks: TrackListView[] = [];
          if (payload.playlist_id === "liked") {
            const paths = [...get(liked).paths];
            tracks = await resolveTracksByPaths(paths);
          } else if (payload.playlist_id === "recent") {
            const files = await invoke<RecentFileListView[]>("get_recent_files", {
              profilId: get(profilSelector).profilSelected?.id ?? 1,
              sourceId: get(sourceFilter),
            });
            const paths = files.map((f) => f.path).filter((x): x is string => !!x);
            tracks = await resolveTracksByPaths(paths);
          } else {
            const plId = Number(payload.playlist_id);
            if (Number.isFinite(plId)) {
              tracks = await invoke<TrackListView[]>("get_playlist_tracks_view", { playlistId: plId });
            }
          }
          if (tracks.length > 0 && tracks[0].path) {
            const startIdx = payload.shuffle ? Math.floor(Math.random() * tracks.length) : 0;
            const startTrack = tracks[startIdx] ?? tracks[0];
            await handlePlayTrack(startTrack.path, toQueue(tracks));
            if (payload.shuffle && !get(queueState).isShuffled) {
              await queueState.toggleShuffle();
            }
            schedulePush(true);
          }
        }
        break;
      case "refresh-playlists":
        await playlistStore.refresh();
        break;
    }
  } catch (e) {
    console.error("[mobile-remote] command error:", cmd, e);
  }
}

export function initMobileRemoteBridge(): void {
  if (initialized) return;
  initialized = true;

  unsubs.push(
    player.subscribe((p) => {
      const sig = `${p.status}|${p.pathFile ?? ""}|${p.duration}|${Math.floor(p.rustPosition ?? 0)}|${p.trackId ?? ""}`;
      if (sig === lastSig) return;
      lastSig = sig;
      schedulePush();
    }),
    queueState.subscribe(() => schedulePush()),
    volume.subscribe(() => schedulePush()),
    liked.subscribe(() => schedulePush()),
    playbackPipelineStore.subscribe(() => schedulePush()),
  );

  void listen<RemoteCmdPayload>("mobile-remote-cmd", (e) => {
    void handleRemoteCmd(e.payload);
  }).then((fn) => {
    unlistenCmd = fn;
  });

  schedulePush(true);
}

export function destroyMobileRemoteBridge(): void {
  if (!initialized) return;
  initialized = false;
  if (unlistenCmd) {
    unlistenCmd();
    unlistenCmd = null;
  }
  unsubs.forEach((u) => u());
  unsubs = [];
  if (pushTimer) {
    clearTimeout(pushTimer);
    pushTimer = null;
  }
}
