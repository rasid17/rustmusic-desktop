<script lang="ts">
  import Icon from "@iconify/svelte";
  import { serverStore } from "#lib/stores/mediaServer/mediaServer.store";
  import { toasts } from "#lib/stores/ui/toast.store";
  import { popinStore } from "#lib/stores/ui/popin.store";
  import { errorMessage } from "#lib/helper/tools/errorTools";
  import { handlePlayTrack } from "#lib/actions/player/PlayerAction";
  import { toQueue } from "#lib/mapper/queue/mapQueueTrack";
  import FormInput from "#lib/components/ui/input/FormInput.svelte";
  import GhostButton from "#lib/components/ui/button/GhostButton.svelte";
  import type { ServerEntry, TidalSearchAlbum, TidalSearchResults, TidalSearchTrack } from "#lib/types/ui/mediaServer/MediaServer";

  let { server }: { server?: ServerEntry } = $props();

  const tidalServers = $derived($serverStore.servers.filter((s) => s.kind === "tidal" && s.enabled));
  let selectedServerId = $state<number | null>(null);
  const activeServerId = $derived(selectedServerId ?? server?.id ?? tidalServers[0]?.id ?? null);

  let query = $state("");
  let tab = $state<"tracks" | "albums">("tracks");
  let searching = $state(false);
  let busyId = $state<string | null>(null);
  let error = $state<string | null>(null);
  let results = $state<TidalSearchResults | null>(null);

  function formatDuration(sec: number | null | undefined): string {
    if (!sec || sec <= 0) return "--:--";
    const m = Math.floor(sec / 60);
    const s = Math.floor(sec % 60);
    return `${m}:${String(s).padStart(2, "0")}`;
  }

  function formatQuality(bits: number | null, rateHz: number | null, hires: boolean): string {
    const khz = rateHz ? (rateHz >= 1000 ? rateHz / 1000 : rateHz) : null;
    if (hires && bits && khz) return `MAX ${bits}-bit / ${khz} kHz`;
    if (hires) return "MAX Hi-Res FLAC";
    if (bits && khz) return `${bits}-bit / ${khz} kHz`;
    return "HiFi Lossless";
  }

  async function doSearch() {
    if (!activeServerId || !query.trim() || searching) return;
    searching = true;
    error = null;
    try {
      results = await serverStore.tidalSearch(activeServerId, query.trim());
    } catch (err) {
      error = errorMessage(err);
    } finally {
      searching = false;
    }
  }

  async function playOrImportTrack(item: TidalSearchTrack, favorite: boolean) {
    if (!activeServerId || busyId) return;
    busyId = `track:${item.id}`;
    error = null;
    try {
      const track = await serverStore.tidalImportTrack(activeServerId, item.id, favorite);
      if (favorite) {
        toasts.push({
          type: "success",
          title: "Added to TIDAL Favorites",
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

  async function importTrackOnly(item: TidalSearchTrack) {
    if (!activeServerId || busyId) return;
    busyId = `fav:${item.id}`;
    error = null;
    try {
      await serverStore.tidalImportTrack(activeServerId, item.id, true);
      toasts.push({
        type: "success",
        title: "Saved to TIDAL Favorites",
        message: `${item.title} — ${item.artist}`,
      });
    } catch (err) {
      error = errorMessage(err);
    } finally {
      busyId = null;
    }
  }

  async function importAlbum(album: TidalSearchAlbum, favorite: boolean, playImmediately: boolean) {
    if (!activeServerId || busyId) return;
    busyId = `album:${album.id}`;
    error = null;
    try {
      const tracks = await serverStore.tidalImportAlbum(activeServerId, album.id, favorite);
      toasts.push({
        type: "success",
        title: favorite ? "Album added to TIDAL Favorites" : "TIDAL Album ready",
        message: `${album.title} · ${tracks.length} tracks (${formatQuality(album.bitDepth, album.samplingRate, album.hires)})`,
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

<div class="flex flex-col gap-4">
  <div class="flex flex-wrap items-center gap-2.5">
    <div class="flex-1 min-w-56">
      <FormInput
        bind:value={query}
        placeholder="Search artist, track, or album on TIDAL..."
        disabled={searching || !activeServerId}
        onenter={doSearch}
      />
    </div>
    <GhostButton
      icon="material-symbols:search-rounded"
      label={searching ? "Searching..." : "Search"}
      busy={searching}
      disabled={!query.trim() || !activeServerId}
      onclick={doSearch}
    />
    {#if tidalServers.length > 1}
      <select
        class="h-9 px-2.5 rounded-lg text-xs border border-(--rg-bd) bg-(--rg-s1) text-(--rg-tx)"
        bind:value={selectedServerId}
      >
        {#each tidalServers as s (s.id)}
          <option value={s.id}>{s.name}</option>
        {/each}
      </select>
    {/if}
  </div>

  <div class="flex items-center justify-between gap-3 border-b border-(--rg-bd) pb-2">
    <div class="flex items-center gap-1.5">
      <button
        type="button"
        class="px-3 py-1.5 rounded-lg text-xs font-semibold cursor-pointer transition-colors
               {tab === 'tracks' ? 'bg-cyan-500/18 text-cyan-700 dark:text-cyan-300 border border-cyan-500/35' : 'text-(--rg-mu) hover:text-(--rg-tx)'}"
        onclick={() => (tab = 'tracks')}
      >
        Tracks {results ? `(${results.tracks.length})` : ""}
      </button>
      <button
        type="button"
        class="px-3 py-1.5 rounded-lg text-xs font-semibold cursor-pointer transition-colors
               {tab === 'albums' ? 'bg-cyan-500/18 text-cyan-700 dark:text-cyan-300 border border-cyan-500/35' : 'text-(--rg-mu) hover:text-(--rg-tx)'}"
        onclick={() => (tab = 'albums')}
      >
        Albums {results ? `(${results.albums.length})` : ""}
      </button>
    </div>
    <span class="text-[11px] text-(--rg-mu) flex items-center gap-1">
      <Icon icon="material-symbols:high-quality-outline-rounded" width="14" class="text-cyan-500" />
      Bit-Perfect MAX up to 24-bit / 192 kHz FLAC
    </span>
  </div>

  {#if error}
    <p class="px-3 py-2 rounded-lg text-xs bg-red-500/12 text-red-500 border border-red-500/25">{error}</p>
  {/if}

  {#if !results && !searching}
    <div class="py-10 text-center text-xs text-(--rg-mu) flex flex-col items-center gap-2">
      <Icon icon="material-symbols:travel-explore-rounded" width="34" class="opacity-45" />
      <p class="font-medium text-(--rg-tx2)">Browse TIDAL Global Hi-Res Catalog</p>
      <p class="max-w-md">
        Enter an artist, track, or album title to play immediately in Bit-Perfect Hi-Res FLAC or add directly to your TIDAL Favorites.
      </p>
    </div>
  {:else if results && tab === "tracks"}
    <div class="max-h-96 overflow-y-auto divide-y divide-(--rg-bd)/60 pr-1">
      {#if results.tracks.length === 0}
        <p class="py-8 text-center text-xs text-(--rg-mu)">No tracks found.</p>
      {/if}
      {#each results.tracks as item (item.id)}
        {@const busy = busyId === `track:${item.id}` || busyId === `fav:${item.id}`}
        <div class="py-2.5 flex items-center gap-3 group">
          {#if item.coverUrl}
            <img src={item.coverUrl} alt={item.title} class="w-11 h-11 rounded-md object-cover shrink-0 bg-(--rg-s2)" />
          {:else}
            <div class="w-11 h-11 rounded-md bg-(--rg-s2) flex items-center justify-center shrink-0 text-(--rg-mu)">
              <Icon icon="material-symbols:music-note-rounded" width="18" />
            </div>
          {/if}
          <div class="flex-1 min-w-0">
            <div class="flex items-center gap-1.5">
              <p class="text-[13px] font-semibold truncate text-(--rg-tx)">{item.title}</p>
              <span class="px-1.5 py-0.5 rounded text-[10px] font-semibold shrink-0
                           {item.hires ? 'bg-cyan-500/15 text-cyan-600 dark:text-cyan-300' : 'bg-(--rg-s2) text-(--rg-mu)'}">
                {formatQuality(item.bitDepth, item.samplingRate, item.hires)}
              </span>
            </div>
            <p class="text-[11.5px] text-(--rg-mu) truncate">
              {item.artist} {item.album ? `· ${item.album}` : ""} · {formatDuration(item.duration)}
            </p>
          </div>
          <div class="flex items-center gap-1.5 shrink-0">
            <button
              type="button"
              disabled={!!busyId}
              onclick={() => importTrackOnly(item)}
              class="h-8 px-2.5 rounded-lg text-xs border border-(--rg-bd) text-(--rg-tx2) hover:bg-(--rg-s2) cursor-pointer disabled:opacity-50"
              title="Add to TIDAL Favorites & sync to library"
            >
              <Icon icon="material-symbols:favorite-outline-rounded" width="15" />
            </button>
            <button
              type="button"
              disabled={!!busyId}
              onclick={() => playOrImportTrack(item, false)}
              class="h-8 px-3 rounded-lg text-xs font-semibold bg-cyan-500/18 text-cyan-700 dark:text-cyan-300 border border-cyan-500/35 hover:bg-cyan-500/28 cursor-pointer disabled:opacity-50 inline-flex items-center gap-1"
            >
              <Icon icon={busy ? "material-symbols:sync-rounded" : "material-symbols:play-arrow-rounded"} width="16" class={busy ? "animate-spin" : ""} />
              Play
            </button>
          </div>
        </div>
      {/each}
    </div>
  {:else if results && tab === "albums"}
    <div class="max-h-96 overflow-y-auto divide-y divide-(--rg-bd)/60 pr-1">
      {#if results.albums.length === 0}
        <p class="py-8 text-center text-xs text-(--rg-mu)">No albums found.</p>
      {/if}
      {#each results.albums as album (album.id)}
        {@const busy = busyId === `album:${album.id}`}
        <div class="py-2.5 flex items-center gap-3">
          {#if album.coverUrl}
            <img src={album.coverUrl} alt={album.title} class="w-12 h-12 rounded-lg object-cover shrink-0 bg-(--rg-s2)" />
          {:else}
            <div class="w-12 h-12 rounded-lg bg-(--rg-s2) flex items-center justify-center shrink-0 text-(--rg-mu)">
              <Icon icon="material-symbols:album-outline" width="20" />
            </div>
          {/if}
          <div class="flex-1 min-w-0">
            <div class="flex items-center gap-1.5">
              <p class="text-[13px] font-semibold truncate text-(--rg-tx)">{album.title}</p>
              <span class="px-1.5 py-0.5 rounded text-[10px] font-semibold shrink-0
                           {album.hires ? 'bg-cyan-500/15 text-cyan-600 dark:text-cyan-300' : 'bg-(--rg-s2) text-(--rg-mu)'}">
                {formatQuality(album.bitDepth, album.samplingRate, album.hires)}
              </span>
            </div>
            <p class="text-[11.5px] text-(--rg-mu) truncate">
              {album.artist} {album.year ? `· ${album.year}` : ""} · {album.tracksCount} tracks
            </p>
          </div>
          <div class="flex items-center gap-1.5 shrink-0">
            <button
              type="button"
              disabled={!!busyId}
              onclick={() => importAlbum(album, true, false)}
              class="h-8 px-2.5 rounded-lg text-xs border border-(--rg-bd) text-(--rg-tx2) hover:bg-(--rg-s2) cursor-pointer disabled:opacity-50"
              title="Add album to TIDAL Favorites & sync"
            >
              <Icon icon="material-symbols:favorite-outline-rounded" width="15" />
            </button>
            <button
              type="button"
              disabled={!!busyId}
              onclick={() => importAlbum(album, false, true)}
              class="h-8 px-3 rounded-lg text-xs font-semibold bg-cyan-500/18 text-cyan-700 dark:text-cyan-300 border border-cyan-500/35 hover:bg-cyan-500/28 cursor-pointer disabled:opacity-50 inline-flex items-center gap-1"
            >
              <Icon icon={busy ? "material-symbols:sync-rounded" : "material-symbols:play-arrow-rounded"} width="16" class={busy ? "animate-spin" : ""} />
              Play Album
            </button>
          </div>
        </div>
      {/each}
    </div>
  {/if}
</div>
