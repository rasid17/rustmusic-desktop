<script lang="ts">
  import Icon from "@iconify/svelte";
  import { t } from "#lib/i18n";
  import { serverStore } from "#lib/stores/mediaServer/mediaServer.store";
  import { toasts } from "#lib/stores/ui/toast.store";
  import { popinStore } from "#lib/stores/ui/popin.store";
  import { errorMessage } from "#lib/helper/tools/errorTools";
  import { handlePlayTrack } from "#lib/actions/player/PlayerAction";
  import { toQueue } from "#lib/mapper/queue/mapQueueTrack";
  import FormInput from "#lib/components/ui/input/FormInput.svelte";
  import GhostButton from "#lib/components/ui/button/GhostButton.svelte";
  import type { QobuzSearchAlbum, QobuzSearchResults, QobuzSearchTrack, ServerEntry } from "#lib/types/ui/mediaServer/MediaServer";

  let { server }: { server?: ServerEntry } = $props();

  const qobuzServers = $derived($serverStore.servers.filter((s) => s.kind === "qobuz" && s.enabled));
  let selectedServerId = $state<number | null>(null);
  const activeServerId = $derived(selectedServerId ?? server?.id ?? qobuzServers[0]?.id ?? null);

  let query = $state("");
  let tab = $state<"tracks" | "albums">("tracks");
  let searching = $state(false);
  let busyId = $state<string | null>(null);
  let error = $state<string | null>(null);
  let results = $state<QobuzSearchResults>({ albums: [], tracks: [] });

  async function search() {
    if (!activeServerId || !query.trim()) return;
    searching = true;
    error = null;
    try {
      results = await serverStore.qobuzSearch(activeServerId, query.trim());
      if (results.tracks.length === 0 && results.albums.length > 0) {
        tab = "albums";
      }
    } catch (err) {
      error = errorMessage(err);
    } finally {
      searching = false;
    }
  }

  function formatDuration(sec: number | null): string {
    if (!sec || sec <= 0) return "";
    const m = Math.floor(sec / 60);
    const s = Math.floor(sec % 60);
    return `${m}:${s.toString().padStart(2, "0")}`;
  }

  function formatQuality(bitDepth: number | null, sampleRate: number | null): string {
    const bd = bitDepth ?? 16;
    const sr = sampleRate ? (sampleRate / 1000).toFixed(sampleRate % 1000 === 0 ? 0 : 1) : "44.1";
    return `${bd}-bit / ${sr} kHz`;
  }

  async function playTrackNow(item: QobuzSearchTrack, favorite = false) {
    if (!activeServerId || busyId) return;
    busyId = `track:${item.id}:${favorite ? "fav" : "play"}`;
    error = null;
    try {
      const track = await serverStore.qobuzImportTrack(activeServerId, item.id, favorite);
      if (favorite) {
        toasts.push({
          type: "success",
          title: $t("streaming.track_fav_added_qobuz"),
          message: `${item.title} — ${item.artist}`,
        });
      }
      if (track?.path) {
        await handlePlayTrack(track.path, toQueue([track]));
      }
    } catch (err) {
      error = errorMessage(err);
    } finally {
      busyId = null;
    }
  }

  async function saveTrackOnly(item: QobuzSearchTrack) {
    if (!activeServerId || busyId) return;
    busyId = `track:${item.id}:fav`;
    error = null;
    try {
      await serverStore.qobuzImportTrack(activeServerId, item.id, true);
      toasts.push({
        type: "success",
        title: $t("streaming.track_saved_qobuz"),
        message: `${item.title} — ${item.artist}`,
      });
    } catch (err) {
      error = errorMessage(err);
    } finally {
      busyId = null;
    }
  }

  async function importAndPlayAlbum(album: QobuzSearchAlbum, playImmediately: boolean, favorite: boolean) {
    if (!activeServerId || busyId) return;
    busyId = `album:${album.id}:${playImmediately ? "play" : "fav"}`;
    error = null;
    try {
      const tracks = await serverStore.qobuzImportAlbum(activeServerId, album.id, favorite);
      toasts.push({
        type: "success",
        title: $t(favorite ? "streaming.album_fav_added_qobuz" : "streaming.album_ready_qobuz"),
        message: $t("streaming.album_summary")
          .replace("{title}", album.title)
          .replace("{n}", String(tracks.length))
          .replace("{quality}", formatQuality(album.bit_depth, album.sampling_rate)),
      });
      if (playImmediately && tracks.length > 0 && tracks[0].path) {
        await handlePlayTrack(tracks[0].path, toQueue(tracks));
        popinStore.close();
      }
    } catch (err) {
      error = errorMessage(err);
    } finally {
      busyId = null;
    }
  }
</script>

<div class="space-y-4">
  <div class="flex flex-wrap items-center gap-2.5">
    <div class="flex-1 min-w-60">
      <FormInput
        id="qobuz_catalog_search"
        bind:value={query}
        placeholder={$t("streaming.qobuz_search_ph")}
        disabled={searching || !activeServerId}
        onenter={search}
      />
    </div>
    <GhostButton
      icon="material-symbols:search-rounded"
      label={searching ? $t("streaming.searching") : $t("streaming.search")}
      busy={searching}
      disabled={searching || !query.trim() || !activeServerId}
      onclick={search}
    />
  </div>

  {#if qobuzServers.length > 1}
    <div class="flex items-center gap-2 text-xs text-(--rg-mu)">
      <span>{$t("streaming.qobuz_account")}</span>
      {#each qobuzServers as s (s.id)}
        <button
          type="button"
          onclick={() => (selectedServerId = s.id)}
          class="px-2.5 py-1 rounded-lg border text-xs font-semibold cursor-pointer transition-colors
                 {activeServerId === s.id ? 'border-(--rg-gbd) bg-(--rg-gbg) text-(--rg-gtx)' : 'border-(--rg-bd) text-(--rg-tx2)'}">
          {s.name}
        </button>
      {/each}
    </div>
  {/if}

  <div class="flex items-center justify-between gap-2 border-b border-(--rg-bd) pb-2">
    <div class="flex items-center gap-1.5">
      <button
        type="button"
        onclick={() => (tab = "tracks")}
        class="px-3 py-1.5 rounded-xl text-xs font-semibold cursor-pointer transition-colors
               {tab === 'tracks' ? 'bg-(--rg-gbg) text-(--rg-gtx) border border-(--rg-gbd)' : 'text-(--rg-mu) hover:text-(--rg-tx)'}">
        {$t("streaming.tracks_tab")} ({results.tracks.length})
      </button>
      <button
        type="button"
        onclick={() => (tab = "albums")}
        class="px-3 py-1.5 rounded-xl text-xs font-semibold cursor-pointer transition-colors
               {tab === 'albums' ? 'bg-(--rg-gbg) text-(--rg-gtx) border border-(--rg-gbd)' : 'text-(--rg-mu) hover:text-(--rg-tx)'}">
        {$t("streaming.albums_tab")} ({results.albums.length})
      </button>
    </div>
    <span class="inline-flex items-center gap-1.5 text-[11px] font-medium text-amber-600 dark:text-amber-400">
      <Icon icon="material-symbols:high-quality-outline-rounded" width="15" />
      {$t("streaming.qobuz_bit_perfect")}
    </span>
  </div>

  {#if error}
    <p class="flex items-center gap-2 px-4 py-3 rounded-xl border border-red-500/25 bg-red-500/8 text-sm text-red-600 dark:text-red-400">
      <Icon icon="material-symbols:warning-outline-rounded" width="17" class="shrink-0" />
      {error}
    </p>
  {/if}

  <div class="max-h-[58vh] overflow-y-auto pr-1 space-y-1.5">
    {#if searching}
      <div class="py-12 flex flex-col items-center justify-center gap-2 text-(--rg-mu)">
        <Icon icon="material-symbols:progress-activity" width="26" class="animate-spin text-(--rg-gtx)" />
        <p class="text-sm">{$t("streaming.qobuz_searching")}</p>
      </div>
    {:else if tab === "tracks"}
      {#each results.tracks as track (track.id)}
        <div class="flex items-center gap-3 px-3 py-2 rounded-xl border border-(--rg-bd) bg-(--rg-s1) hover:border-(--rg-bd2) transition-colors">
          {#if track.cover_url}
            <img src={track.cover_url} alt="" class="w-11 h-11 rounded-lg object-cover shrink-0 bg-(--rg-s2)" loading="lazy" />
          {:else}
            <div class="w-11 h-11 rounded-lg shrink-0 bg-(--rg-s2) flex items-center justify-center text-(--rg-mu)">
              <Icon icon="material-symbols:music-note-rounded" width="20" />
            </div>
          {/if}
          <div class="flex-1 min-w-0">
            <div class="flex items-center gap-2">
              <p class="text-sm font-semibold truncate text-(--rg-tx)">{track.title}</p>
              <span class="shrink-0 px-1.5 py-0.5 rounded text-[10px] font-bold uppercase tracking-wider
                           {track.hires ? 'bg-amber-500/15 text-amber-600 dark:text-amber-300 border border-amber-500/30' : 'bg-(--rg-s2) text-(--rg-mu)'}">
                {formatQuality(track.bit_depth, track.sampling_rate)}
              </span>
            </div>
            <p class="text-xs truncate text-(--rg-mu)">
              {[track.artist, track.album, track.year, formatDuration(track.duration)].filter(Boolean).join(" · ")}
            </p>
          </div>
          <div class="flex items-center gap-1.5 shrink-0">
            <GhostButton
              icon="material-symbols:play-arrow-rounded"
              label={$t("streaming.play")}
              busy={busyId === `track:${track.id}:play`}
              disabled={busyId !== null}
              onclick={() => playTrackNow(track, false)}
            />
            <GhostButton
              icon="material-symbols:favorite-outline-rounded"
              label={$t("streaming.favorite")}
              busy={busyId === `track:${track.id}:fav`}
              disabled={busyId !== null}
              onclick={() => saveTrackOnly(track)}
            />
          </div>
        </div>
      {:else}
        <p class="py-10 text-center text-sm text-(--rg-mu)">
          {$t("streaming.qobuz_empty_tracks")}
        </p>
      {/each}
    {:else}
      {#each results.albums as album (album.id)}
        <div class="flex items-center gap-3.5 px-3 py-2.5 rounded-xl border border-(--rg-bd) bg-(--rg-s1) hover:border-(--rg-bd2) transition-colors">
          {#if album.cover_url}
            <img src={album.cover_url} alt="" class="w-14 h-14 rounded-xl object-cover shrink-0 bg-(--rg-s2)" loading="lazy" />
          {:else}
            <div class="w-14 h-14 rounded-xl shrink-0 bg-(--rg-s2) flex items-center justify-center text-(--rg-mu)">
              <Icon icon="material-symbols:album-outline-rounded" width="24" />
            </div>
          {/if}
          <div class="flex-1 min-w-0">
            <div class="flex items-center gap-2">
              <p class="text-sm font-semibold truncate text-(--rg-tx)">{album.title}</p>
              <span class="shrink-0 px-1.5 py-0.5 rounded text-[10px] font-bold uppercase tracking-wider
                           {album.hires ? 'bg-amber-500/15 text-amber-600 dark:text-amber-300 border border-amber-500/30' : 'bg-(--rg-s2) text-(--rg-mu)'}">
                {formatQuality(album.bit_depth, album.sampling_rate)}
              </span>
            </div>
            <p class="text-xs truncate text-(--rg-mu)">
              {[album.artist, album.year, $t("streaming.tracks_unit").replace("{n}", String(album.tracks_count)), album.genre].filter(Boolean).join(" · ")}
            </p>
          </div>
          <div class="flex items-center gap-1.5 shrink-0">
            <GhostButton
              icon="material-symbols:play-arrow-rounded"
              label={$t("streaming.play_album")}
              busy={busyId === `album:${album.id}:play`}
              disabled={busyId !== null}
              onclick={() => importAndPlayAlbum(album, true, false)}
            />
            <GhostButton
              icon="material-symbols:favorite-outline-rounded"
              label={$t("streaming.favorite")}
              busy={busyId === `album:${album.id}:fav`}
              disabled={busyId !== null}
              onclick={() => importAndPlayAlbum(album, false, true)}
            />
          </div>
        </div>
      {:else}
        <p class="py-10 text-center text-sm text-(--rg-mu)">
          {$t("streaming.qobuz_empty_albums")}
        </p>
      {/each}
    {/if}
  </div>
</div>
