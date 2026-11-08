<script lang="ts">
  import { tick } from "svelte";
  import { fade, scale } from "svelte/transition";
  import Icon from "@iconify/svelte";
  import { goto } from "$app/navigation";
  import { invoke } from "@tauri-apps/api/core";
  import { commandPaletteStore } from "#lib/stores/ui/commandPalette.store";
  import { libraryStore } from "#lib/stores/library/library.store";
  import { libraryContentStore } from "#lib/stores/library/libraryContent.store";
  import { importsTermines } from "#lib/stores/library/importProgress.store";
  import { playlistStore } from "#lib/stores/playlist/playlist.store";
  import { profilSelector } from "#lib/stores/profil/profil.store";
  import { sourceFilter } from "#lib/stores/source/sourceFilter.store";
  import { currentSource, activeServers } from "#lib/stores/source/source.store";
  import { settingsStore } from "#lib/stores/settings/settings.store";
  import { queueState } from "#lib/stores/queue/queueState.store";
  import { toasts } from "#lib/stores/ui/toast.store";
  import { popinStore } from "#lib/stores/ui/popin.store";
  import { toggleMiniPlayer } from "#lib/stores/ui/miniPlayer.store";
  import { playerService } from "#lib/services/player/player.service";
  import { handlePlayTrack } from "#lib/actions/player/PlayerAction";
  import { toQueue, toQueueTrack } from "#lib/mapper/queue/mapQueueTrack";
  import { groupVersions, type SearchResult, type SearchHit } from "#lib/helper/search/searchVersions";
  import { forgetLocations } from "#lib/helper/library/trackLocation";
  import { getFolderProfiles, applyFolderProfile, type FolderProfile } from "#lib/helper/library/folderProfiles";
  import type { LibraryDir } from "#lib/types/db/library/LibraryDir";
  import type { Playlist } from "#lib/types/db/playlist/Playlist";
  import AiPlaylistPopin from "#lib/components/playlist/popin/AiPlaylistPopin.svelte";
  import CoverImg from "#lib/components/ui/image/CoverImg.svelte";
  import CloudMark from "#lib/components/ui/status/CloudMark.svelte";
  import VersionMarks from "#lib/components/ui/status/VersionMarks.svelte";

  type PaletteCommand = {
    kind: "command";
    id: string;
    category: string;
    title: string;
    subtitle?: string;
    icon: string;
    badge?: string;
    badgeVariant?: "green" | "gray" | "blue" | "purple";
    action: () => void | Promise<void>;
  };

  type PaletteTrack = {
    kind: "track";
    id: string;
    category: string;
    hit: SearchHit;
  };

  type PaletteAlbum = {
    kind: "album";
    id: string;
    category: string;
    hit: SearchHit;
  };

  type PaletteArtist = {
    kind: "artist";
    id: string;
    category: string;
    hit: SearchHit;
  };

  type PalettePlaylist = {
    kind: "playlist";
    id: string;
    category: string;
    playlist: Playlist;
  };

  type PaletteEntry = PaletteCommand | PaletteTrack | PaletteAlbum | PaletteArtist | PalettePlaylist;

  let inputEl = $state<HTMLInputElement | null>(null);
  let listEl = $state<HTMLDivElement | null>(null);

  let query = $state("");
  let selectedIndex = $state(0);
  let isSearching = $state(false);
  let searchResults = $state<SearchResult[]>([]);
  let searchDebounceTimer: ReturnType<typeof setTimeout> | null = null;

  let dirs = $state<LibraryDir[]>([]);
  let profiles = $state<FolderProfile[]>([]);
  let devices = $state<string[]>([]);

  let pressedOnBackdrop = false;

  const libraryId = $derived($libraryStore.librarySelected?.id ?? null);

  const norm = (s: string | null | undefined) =>
    (s ?? "").normalize("NFD").replace(/\p{M}/gu, "").toLowerCase().replace(/\s+/g, " ").trim();

  // Load contextual data (dirs, profiles, audio devices)
  async function loadContextData() {
    const libId = $libraryStore.librarySelected?.id ?? null;
    if (libId) {
      try {
        dirs = await invoke<LibraryDir[]>("get_library_dirs", { libraryId: libId });
      } catch (e) {
        console.warn("Failed to load library dirs:", e);
        dirs = [];
      }
      try {
        profiles = getFolderProfiles(libId);
      } catch (e) {
        console.warn("Failed to load folder profiles:", e);
        profiles = [];
      }
    } else {
      dirs = [];
      profiles = [];
    }

    try {
      const devs = await invoke<string[]>("get_devices");
      devices = Array.isArray(devs) ? devs : [];
    } catch (e) {
      console.warn("Failed to load audio devices:", e);
      devices = [];
    }
  }

  // Open & close reactive effect
  $effect(() => {
    if ($commandPaletteStore.open) {
      query = $commandPaletteStore.initialQuery || "";
      selectedIndex = 0;
      searchResults = [];
      void loadContextData();
      if (query.trim()) {
        triggerSearch(query);
      }
      tick().then(() => {
        inputEl?.focus();
        inputEl?.select();
      });
    } else {
      if (searchDebounceTimer) clearTimeout(searchDebounceTimer);
      query = "";
      searchResults = [];
      selectedIndex = 0;
    }
  });

  // Action handlers
  async function handleToggleDir(dir: LibraryDir) {
    const libId = libraryId;
    if (!libId) return;
    try {
      await invoke("set_library_dir_active", { dirId: dir.id, isActive: !dir.is_active });
      forgetLocations();
      await Promise.all([
        libraryContentStore.refresh(libId),
        libraryStore.refresh(),
      ]);
      importsTermines.update((n) => n + 1);
      toasts.push({
        type: "success",
        title: "Folder Tersinkronisasi",
        message: `${dir.name}: ${!dir.is_active ? "Diaktifkan" : "Dinonaktifkan"}`,
      });
      await loadContextData();
    } catch (e) {
      toasts.push({ type: "error", title: "Gagal Mengubah Folder", message: String(e) });
    }
  }

  async function handleApplyProfile(profile: FolderProfile) {
    const libId = libraryId;
    if (!libId) return;
    try {
      await applyFolderProfile(libId, profile.activeDirIds);
      toasts.push({
        type: "success",
        title: "Mode Dengar",
        message: `Profil "${profile.name}" diterapkan`,
      });
      await loadContextData();
    } catch (e) {
      toasts.push({ type: "error", title: "Gagal Menerapkan Profil", message: String(e) });
    }
  }

  async function handleActivateAllDirs() {
    const libId = libraryId;
    if (!libId || dirs.length === 0) return;
    try {
      await applyFolderProfile(libId, dirs.map((d) => d.id));
      toasts.push({
        type: "success",
        title: "Mode Dengar",
        message: "Semua folder diaktifkan",
      });
      await loadContextData();
    } catch (e) {
      toasts.push({ type: "error", title: "Gagal Mengaktifkan Folder", message: String(e) });
    }
  }

  async function handleSetDevice(deviceName: string) {
    try {
      await invoke("set_device", { deviceName });
      toasts.push({
        type: "success",
        title: "Audio Output",
        message: `Perangkat diubah ke: ${deviceName}`,
      });
    } catch (e) {
      toasts.push({
        type: "error",
        title: "Audio Output",
        message: `Gagal mengubah perangkat: ${String(e)}`,
      });
    }
  }

  // Prepares command entries
  const allCommandEntries = $derived.by((): PaletteCommand[] => {
    const libId = libraryId;
    const list: PaletteCommand[] = [
      // AI Playlist
      {
        kind: "command",
        id: "ai_new_playlist",
        category: "AI Playlist",
        title: "AI Playlist Generator: Buat Playlist Baru...",
        subtitle: "Hasilkan playlist otomatis berbasis prompt cerdas",
        icon: "mynaui:sparkles",
        badge: "AI",
        badgeVariant: "purple",
        action: () => {
          popinStore.open(
            "AI Playlist Generator",
            AiPlaylistPopin,
            { libraryId: libId },
            { size: "lg", icon: "mynaui:sparkles" }
          );
        },
      },
      // Playback
      {
        kind: "command",
        id: "playback_toggle_play",
        category: "Pemutaran",
        title: "Play / Pause",
        subtitle: "Putar atau jeda lagu saat ini",
        icon: "lucide:play",
        action: () => playerService.handleTogglePlay(),
      },
      {
        kind: "command",
        id: "playback_next",
        category: "Pemutaran",
        title: "Next Track",
        subtitle: "Lompat ke lagu berikutnya",
        icon: "lucide:skip-forward",
        action: () => playerService.nextTrack(),
      },
      {
        kind: "command",
        id: "playback_prev",
        category: "Pemutaran",
        title: "Previous Track",
        subtitle: "Kembali ke lagu sebelumnya",
        icon: "lucide:skip-back",
        action: () => playerService.prevTrack(),
      },
      {
        kind: "command",
        id: "playback_shuffle",
        category: "Pemutaran",
        title: "Smart Shuffle: Toggle / Reshuffle Queue",
        subtitle: $queueState.isShuffled ? "Acak ulang antrean saat ini" : "Aktifkan putar acak cerdas",
        icon: "lucide:shuffle",
        action: async () => {
          await queueState.toggleShuffle();
          toasts.push({
            type: "info",
            title: "Smart Shuffle",
            message: !$queueState.isShuffled ? "Smart Shuffle diaktifkan" : "Smart Shuffle dinonaktifkan",
          });
        },
      },
      {
        kind: "command",
        id: "playback_clear_queue",
        category: "Pemutaran",
        title: "Clear Queue",
        subtitle: "Hapus seluruh antrean pemutaran",
        icon: "lucide:trash-2",
        action: async () => {
          await queueState.clear();
          toasts.push({
            type: "info",
            title: "Antrean",
            message: "Antrean dibersihkan",
          });
        },
      },
      {
        kind: "command",
        id: "playback_mini_player",
        category: "Pemutaran",
        title: "Toggle Mini Player (Picture-in-Picture)",
        subtitle: "Buka atau tutup jendela pemutar mini",
        icon: "lucide:picture-in-picture-2",
        action: () => {
          void toggleMiniPlayer();
        },
      },
      // Navigation & Appearance
      {
        kind: "command",
        id: "nav_theme",
        category: "Tampilan & Navigasi",
        title: "Toggle Theme (Light / Dark)",
        subtitle: "Beralih antara tema gelap dan terang",
        icon: "lucide:sun-moon",
        action: async () => {
          const isDark = document.documentElement.classList.contains("dark");
          const nextTheme = isDark ? "light" : "dark";
          await settingsStore.set("theme", nextTheme);
          toasts.push({
            type: "info",
            title: "Tema",
            message: isDark ? "Mode Terang diaktifkan" : "Mode Gelap diaktifkan",
          });
        },
      },
      {
        kind: "command",
        id: "nav_playlists",
        category: "Tampilan & Navigasi",
        title: "Go to Playlists",
        subtitle: "Buka daftar semua playlist",
        icon: "lucide:list-music",
        action: () => {
          goto("/playlists");
        },
      },
      {
        kind: "command",
        id: "nav_favorites",
        category: "Tampilan & Navigasi",
        title: "Go to Favorites",
        subtitle: "Buka daftar lagu yang disukai",
        icon: "lucide:heart",
        action: () => {
          goto("/playlist/liked");
        },
      },
      {
        kind: "command",
        id: "nav_settings",
        category: "Tampilan & Navigasi",
        title: "Go to Settings",
        subtitle: "Buka pengaturan umum aplikasi",
        icon: "lucide:settings",
        action: () => {
          goto("/settings/general");
        },
      },
      {
        kind: "command",
        id: "nav_podcasts",
        category: "Tampilan & Navigasi",
        title: "Go to Podcasts",
        subtitle: "Buka daftar saluran dan episode podcast",
        icon: "lucide:podcast",
        action: () => {
          goto("/podcasts");
        },
      },
    ];

    // Folder Profiles
    if (libId) {
      if (dirs.length > 0) {
        list.push({
          kind: "command",
          id: "folder_profile_all",
          category: "Mode Dengar (Folder)",
          title: "Folder Profile: Aktifkan Semua Folder",
          subtitle: `Aktifkan semua ${dirs.length} folder tersinkronisasi`,
          icon: "lucide:folder-check",
          action: () => handleActivateAllDirs(),
        });
      }

      for (const prof of profiles) {
        list.push({
          kind: "command",
          id: `folder_profile_${prof.id}`,
          category: "Mode Dengar (Folder)",
          title: `Folder Profile: ${prof.name}`,
          subtitle: `${prof.activeDirIds.length} folder aktif`,
          icon: "lucide:folder-heart",
          action: () => handleApplyProfile(prof),
        });
      }

      // Synced Folders
      for (const dir of dirs) {
        list.push({
          kind: "command",
          id: `dir_${dir.id}`,
          category: "Folder Tersinkronisasi",
          title: `Folder: ${dir.is_active ? "Nonaktifkan" : "Aktifkan"} ${dir.name}`,
          subtitle: dir.path,
          icon: dir.is_active ? "lucide:folder-minus" : "lucide:folder-plus",
          badge: dir.is_active ? "Aktif" : "Nonaktif",
          badgeVariant: dir.is_active ? "green" : "gray",
          action: () => handleToggleDir(dir),
        });
      }

      list.push({
        kind: "command",
        id: "dir_manage",
        category: "Folder Tersinkronisasi",
        title: "Kelola Synced Folders...",
        subtitle: "Buka pengaturan folder perpustakaan",
        icon: "lucide:folder-cog",
        action: () => {
          goto(`/library/${libId}/folders`);
        },
      });
    }

    // Audio Output Devices
    for (const device of devices) {
      list.push({
        kind: "command",
        id: `device_${device}`,
        category: "Audio Output",
        title: `Audio Output: ${device}`,
        subtitle: "Alihkan perangkat keluaran audio",
        icon: "lucide:speaker",
        action: () => handleSetDevice(device),
      });
    }

    return list;
  });

  // Grouped search results
  const groupedHits = $derived(
    groupVersions(
      searchResults,
      $libraryStore.libraries,
      $currentSource === "all" && $activeServers.length > 0,
      $settingsStore.duplicate_version === "server"
    )
  );

  // Matching playlists
  const matchingPlaylists = $derived(
    !query.trim().startsWith(">") && query.trim().length >= 2
      ? $playlistStore.playlists.filter((p) => norm(p.name).includes(norm(query.trim())))
      : []
  );

  function isAiPrompt(q: string): string | null {
    const trimmed = q.trim();
    if (trimmed.startsWith("> ai ")) return trimmed.slice(5).trim();
    if (trimmed.startsWith(">ai ")) return trimmed.slice(4).trim();
    if (trimmed.toLowerCase().startsWith("ai: ")) return trimmed.slice(4).trim();
    if (trimmed.toLowerCase().startsWith("ai:")) return trimmed.slice(3).trim();
    return null;
  }

  // Unified list of entries
  const flattenedEntries = $derived.by((): PaletteEntry[] => {
    const list: PaletteEntry[] = [];
    const q = query.trim();

    // 1. Check for AI prompt prefix
    const aiPrompt = isAiPrompt(q);
    if (aiPrompt) {
      const libId = libraryId;
      list.push({
        kind: "command",
        id: "ai_custom_generate",
        category: "AI Playlist",
        title: `Generate AI Playlist: "${aiPrompt}"`,
        subtitle: "Buat playlist otomatis dengan prompt cerdas ini",
        icon: "mynaui:sparkles",
        badge: "AI",
        badgeVariant: "purple",
        action: () => {
          popinStore.open(
            "AI Playlist Generator",
            AiPlaylistPopin,
            { libraryId: libId, initialPrompt: aiPrompt },
            { size: "lg", icon: "mynaui:sparkles" }
          );
        },
      });
    }

    // 2. Search mode (not starting with ">" and query length >= 2)
    if (!q.startsWith(">") && q.length >= 2) {
      // Matching Playlists
      for (const pl of matchingPlaylists) {
        list.push({
          kind: "playlist",
          id: `pl_${pl.id}`,
          category: "Playlist",
          playlist: pl,
        });
      }

      // Tracks
      for (const hit of groupedHits.filter((h) => h.result_type === "track")) {
        list.push({
          kind: "track",
          id: `track_${hit.id}`,
          category: "Lagu",
          hit,
        });
      }

      // Albums
      for (const hit of groupedHits.filter((h) => h.result_type === "album")) {
        list.push({
          kind: "album",
          id: `album_${hit.id}`,
          category: "Album",
          hit,
        });
      }

      // Artists
      for (const hit of groupedHits.filter((h) => h.result_type === "artist")) {
        list.push({
          kind: "artist",
          id: `artist_${hit.id}`,
          category: "Artis",
          hit,
        });
      }

      // Matching Commands
      const needle = norm(q);
      const matchingCmds = allCommandEntries.filter((cmd) => {
        return (
          norm(cmd.title).includes(needle) ||
          (cmd.subtitle && norm(cmd.subtitle).includes(needle)) ||
          norm(cmd.category).includes(needle)
        );
      });
      list.push(...matchingCmds);

      return list;
    }

    // 3. Command mode ("> ...") or Initial suggestions (empty or 1 character)
    const cmdFilter = q.startsWith(">") ? norm(q.slice(1).trim()) : norm(q);

    const filteredCmds =
      cmdFilter === ""
        ? allCommandEntries
        : allCommandEntries.filter(
            (cmd) =>
              norm(cmd.title).includes(cmdFilter) ||
              (cmd.subtitle && norm(cmd.subtitle).includes(cmdFilter)) ||
              norm(cmd.category).includes(cmdFilter)
          );

    list.push(...filteredCmds);
    return list;
  });

  // Keep selected index in bounds when list changes
  $effect(() => {
    if (flattenedEntries.length === 0) {
      selectedIndex = 0;
    } else if (selectedIndex >= flattenedEntries.length) {
      selectedIndex = Math.max(0, flattenedEntries.length - 1);
    }
  });

  function scrollToSelected() {
    tick().then(() => {
      if (!listEl) return;
      const el = listEl.querySelector<HTMLElement>(`[data-palette-index="${selectedIndex}"]`);
      el?.scrollIntoView({ block: "nearest" });
    });
  }

  function handleInput() {
    selectedIndex = 0;
    triggerSearch(query);
  }

  function triggerSearch(q: string) {
    if (searchDebounceTimer) clearTimeout(searchDebounceTimer);
    const trimmed = q.trim();
    if (trimmed.startsWith(">") || trimmed.length < 2) {
      searchResults = [];
      isSearching = false;
      return;
    }
    isSearching = true;
    searchDebounceTimer = setTimeout(async () => {
      try {
        const res = await invoke<SearchResult[]>("search", {
          query: trimmed,
          limit: 15,
          profilId: $profilSelector.profilSelected?.id ?? 0,
          sourceId: $sourceFilter,
        });
        if (query.trim() === trimmed) {
          searchResults = res;
        }
      } catch (e) {
        console.error("Search failed in CommandPalette:", e);
        if (query.trim() === trimmed) {
          searchResults = [];
        }
      } finally {
        if (query.trim() === trimmed) {
          isSearching = false;
        }
      }
    }, 120);
  }

  function executeEntry(item: PaletteEntry) {
    if (item.kind === "command") {
      void item.action();
      close();
    } else if (item.kind === "track") {
      if (item.hit.path) {
        const allTracks = flattenedEntries
          .filter((e): e is PaletteTrack => e.kind === "track")
          .map((e) => e.hit);
        handlePlayTrack(item.hit.path, toQueue(allTracks));
      }
      close();
    } else if (item.kind === "album") {
      if (item.hit.library_id) {
        goto(`/library/${item.hit.library_id}/albums/${item.hit.id}`);
      }
      close();
    } else if (item.kind === "artist") {
      if (item.hit.library_id) {
        goto(`/library/${item.hit.library_id}/artists/${item.hit.id}`);
      }
      close();
    } else if (item.kind === "playlist") {
      goto(`/playlist/${item.playlist.id}`);
      close();
    }
  }

  function openVersion(hit: SearchHit, r: SearchResult) {
    if (r.result_type === "track" && r.path) {
      const pistes = groupedHits
        .filter((x) => x.result_type === "track" && x.path)
        .map((x) => (x === hit ? { ...r, versions: hit.versions } : x));
      handlePlayTrack(r.path, toQueue(pistes));
      close();
    } else {
      executeEntry({
        kind: r.result_type as "album" | "artist",
        id: r.id,
        category: r.result_type === "album" ? "Album" : "Artis",
        hit: { ...r, versions: hit.versions },
      });
    }
  }

  function handleKeydown(e: KeyboardEvent) {
    if (e.key === "ArrowDown") {
      e.preventDefault();
      if (flattenedEntries.length > 0) {
        selectedIndex = (selectedIndex + 1) % flattenedEntries.length;
        scrollToSelected();
      }
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      if (flattenedEntries.length > 0) {
        selectedIndex = (selectedIndex - 1 + flattenedEntries.length) % flattenedEntries.length;
        scrollToSelected();
      }
    } else if (e.key === "Enter") {
      e.preventDefault();
      if (flattenedEntries.length === 0) return;
      const item = flattenedEntries[selectedIndex];
      if (!item) return;

      if (e.shiftKey) {
        // Shift+Enter: Play Next on a track
        if (item.kind === "track" && item.hit.path) {
          void queueState.addTrack(toQueueTrack(item.hit));
          toasts.push({ type: "success", title: "Ditambahkan diputar berikutnya", message: item.hit.title });
          close();
        } else {
          executeEntry(item);
        }
        return;
      }

      if (e.ctrlKey || e.metaKey) {
        // Ctrl+Enter / Meta+Enter: Enqueue on a track, keeps palette open
        if (item.kind === "track" && item.hit.path) {
          void queueState.enqueue(toQueueTrack(item.hit));
          toasts.push({ type: "success", title: "Ditambahkan ke antrean", message: item.hit.title });
        } else {
          executeEntry(item);
        }
        return;
      }

      executeEntry(item);
    } else if (e.key === "Escape") {
      e.preventDefault();
      close();
    }
  }

  function close() {
    commandPaletteStore.close();
  }

  function onBackdropPointerDown(e: MouseEvent) {
    pressedOnBackdrop = e.target === e.currentTarget;
  }

  function onBackdropClick(e: MouseEvent) {
    if (pressedOnBackdrop && e.target === e.currentTarget) {
      close();
    }
    pressedOnBackdrop = false;
  }
</script>

{#if $commandPaletteStore.open}
  <!-- Backdrop -->
  <div
    role="presentation"
    class="fixed inset-0 z-50 flex items-start justify-center pt-[10vh] px-4 bg-black/60 backdrop-blur-sm"
    transition:fade={{ duration: 120 }}
    onmousedown={onBackdropPointerDown}
    onclick={onBackdropClick}
  >
    <!-- Modal Dialog -->
    <div
      role="dialog"
      aria-modal="true"
      aria-label="Command Palette"
      class="flex flex-col w-full max-w-2xl max-h-[75vh] rounded-2xl overflow-hidden shadow-2xl shadow-black/50
             border border-neutral-200/80 dark:border-neutral-800
             bg-white dark:bg-[#121614] text-neutral-900 dark:text-neutral-100"
      transition:scale={{ start: 0.97, duration: 120 }}
    >
      <!-- Search & Command Input Bar -->
      <div
        class="flex items-center gap-3 px-4 py-3.5 border-b border-neutral-200 dark:border-neutral-800/80
               bg-neutral-50/50 dark:bg-[#151a17]/50"
      >
        {#if isSearching}
          <Icon icon="lucide:loader-2" class="w-5 h-5 animate-spin text-emerald-500 shrink-0" />
        {:else if query.trim().startsWith(">")}
          <Icon icon="lucide:terminal" class="w-5 h-5 text-emerald-500 shrink-0" />
        {:else}
          <Icon icon="mynaui:search" class="w-5 h-5 text-neutral-400 shrink-0" />
        {/if}

        <input
          bind:this={inputEl}
          type="text"
          bind:value={query}
          oninput={handleInput}
          onkeydown={handleKeydown}
          placeholder="Ketik perintah ('>') atau cari lagu, album, artis, playlist..."
          class="flex-1 bg-transparent border-none outline-none text-sm text-neutral-900 dark:text-neutral-100
                 placeholder:text-neutral-400 dark:placeholder:text-neutral-500"
          autocomplete="off"
          spellcheck="false"
        />

        {#if query}
          <button
            type="button"
            onclick={() => { query = ""; handleInput(); }}
            class="p-1 rounded-md text-neutral-400 hover:text-neutral-600 dark:hover:text-neutral-200
                   hover:bg-neutral-200/60 dark:hover:bg-neutral-800 transition-colors"
            title="Hapus pencarian"
          >
            <Icon icon="lucide:x" class="w-4 h-4" />
          </button>
        {/if}

        <kbd
          class="hidden sm:inline-flex items-center px-1.5 py-0.5 rounded border border-neutral-200 dark:border-neutral-700
                 bg-neutral-100 dark:bg-neutral-800 text-[10.5px] font-mono text-neutral-500 dark:text-neutral-400 select-none"
        >
          ESC
        </kbd>
      </div>

      <!-- Results & Commands List -->
      <div
        bind:this={listEl}
        role="listbox"
        tabindex="-1"
        class="flex-1 overflow-y-auto min-h-[160px] max-h-[460px] p-2 space-y-0.5 focus:outline-none"
      >
        {#if flattenedEntries.length === 0}
          <div class="flex flex-col items-center justify-center py-12 px-4 text-center">
            {#if isSearching}
              <Icon icon="lucide:loader-2" class="w-8 h-8 animate-spin text-emerald-500 mb-2" />
              <p class="text-sm text-neutral-500 dark:text-neutral-400">Mencari di perpustakaan...</p>
            {:else if query.trim().length >= 2}
              <Icon icon="lucide:search-x" class="w-8 h-8 text-neutral-400 mb-2" />
              <p class="text-sm font-medium text-neutral-700 dark:text-neutral-300">
                Tidak ada hasil untuk "{query}"
              </p>
              <p class="text-xs text-neutral-400 dark:text-neutral-500 mt-1">
                Ketik <span class="font-mono bg-neutral-100 dark:bg-neutral-800 px-1 py-0.5 rounded">&gt;</span> untuk melihat semua perintah cepat
              </p>
            {:else}
              <Icon icon="lucide:command" class="w-8 h-8 text-neutral-400 mb-2" />
              <p class="text-sm font-medium text-neutral-700 dark:text-neutral-300">
                Tidak ada tindakan atau perintah yang cocok
              </p>
            {/if}
          </div>
        {:else}
          {#each flattenedEntries as item, i (item.id)}
            <!-- Category Header -->
            {#if i === 0 || flattenedEntries[i - 1].category !== item.category}
              <div
                class="px-3 pt-3 pb-1 text-[10.5px] font-semibold uppercase tracking-wider text-neutral-400 dark:text-neutral-500 select-none"
              >
                {item.category}
              </div>
            {/if}

            <!-- Entry Row -->
            <div
              role="option"
              tabindex="-1"
              aria-selected={i === selectedIndex}
              data-palette-index={i}
              onclick={() => executeEntry(item)}
              onkeydown={(e) => { if (e.key === "Enter") executeEntry(item); }}
              onmouseenter={() => { selectedIndex = i; }}
              class="group flex items-center gap-3 px-3 py-2 rounded-xl cursor-pointer transition-colors text-left
                     {i === selectedIndex
                       ? 'bg-emerald-500/10 dark:bg-emerald-500/15 text-emerald-950 dark:text-emerald-50'
                       : 'hover:bg-neutral-100/70 dark:hover:bg-white/5 text-neutral-800 dark:text-neutral-200'}"
            >
              <!-- Icon or Thumbnail -->
              {#if item.kind === "command"}
                <div
                  class="w-8 h-8 rounded-lg flex items-center justify-center shrink-0
                         bg-emerald-500/10 dark:bg-emerald-500/20 text-emerald-600 dark:text-emerald-400"
                >
                  <Icon icon={item.icon} class="w-4 h-4" />
                </div>
              {:else if item.kind === "playlist"}
                <div
                  class="w-8 h-8 rounded-lg flex items-center justify-center shrink-0
                         bg-purple-500/10 dark:bg-purple-500/20 text-purple-600 dark:text-purple-400"
                >
                  <Icon icon="lucide:list-music" class="w-4 h-4" />
                </div>
              {:else if item.hit.thumbnail_path}
                <CoverImg
                  path={item.hit.thumbnail_path}
                  alt=""
                  class="w-8 h-8 rounded-lg object-cover shrink-0"
                />
              {:else}
                <div
                  class="w-8 h-8 rounded-lg flex items-center justify-center shrink-0
                         bg-neutral-100 dark:bg-neutral-800 text-neutral-400"
                >
                  <Icon
                    icon={item.kind === "track"
                      ? "lucide:music"
                      : item.kind === "album"
                      ? "lucide:disc-album"
                      : "lucide:mic-2"}
                    class="w-4 h-4"
                  />
                </div>
              {/if}

              <!-- Text Details -->
              <div class="flex-1 min-w-0">
                <div class="flex items-center gap-1.5 text-sm font-medium truncate">
                  {#if item.kind === "track"}
                    {#if item.hit.versions.length > 1}
                      <VersionMarks
                        versions={item.hit.versions}
                        onopen={(r) => openVersion(item.hit, r)}
                        size={12}
                      />
                    {:else}
                      <CloudMark path={item.hit.path} libraryId={item.hit.library_id} size={12} />
                    {/if}
                    <span class="truncate">{item.hit.title}</span>
                  {:else if item.kind === "album" || item.kind === "artist"}
                    <span class="truncate">{item.hit.title}</span>
                  {:else if item.kind === "playlist"}
                    <span class="truncate">{item.playlist.name}</span>
                  {:else}
                    <span class="truncate">{item.title}</span>
                  {/if}
                </div>

                {#if item.kind === "command" && item.subtitle}
                  <p class="text-xs text-neutral-500 dark:text-neutral-400 truncate">
                    {item.subtitle}
                  </p>
                {:else if item.kind === "track" && item.hit.subtitle}
                  <p class="text-xs text-neutral-500 dark:text-neutral-400 truncate">
                    {item.hit.subtitle}
                  </p>
                {:else if item.kind === "album" && item.hit.subtitle}
                  <p class="text-xs text-neutral-500 dark:text-neutral-400 truncate">
                    {item.hit.subtitle}
                  </p>
                {:else if item.kind === "playlist"}
                  <p class="text-xs text-neutral-500 dark:text-neutral-400 truncate">
                    {item.playlist.track_count ?? 0} lagu
                  </p>
                {/if}
              </div>

              <!-- Action buttons for Track row -->
              {#if item.kind === "track"}
                <div
                  class="flex items-center gap-1 shrink-0 transition-opacity
                         {i === selectedIndex ? 'opacity-100' : 'opacity-0 group-hover:opacity-100'}"
                >
                  <button
                    type="button"
                    title="Putar Berikutnya (Shift+Enter)"
                    onclick={(e) => {
                      e.stopPropagation();
                      if (item.hit.path) {
                        void queueState.addTrack(toQueueTrack(item.hit));
                        toasts.push({
                          type: "success",
                          title: "Ditambahkan diputar berikutnya",
                          message: item.hit.title,
                        });
                        close();
                      }
                    }}
                    class="p-1 rounded-md text-neutral-400 hover:text-emerald-500 hover:bg-neutral-200/60 dark:hover:bg-white/10 transition-colors"
                  >
                    <Icon icon="lucide:list-plus" class="w-4 h-4" />
                  </button>
                  <button
                    type="button"
                    title="Tambahkan ke antrean (Ctrl+Enter)"
                    onclick={(e) => {
                      e.stopPropagation();
                      if (item.hit.path) {
                        void queueState.enqueue(toQueueTrack(item.hit));
                        toasts.push({
                          type: "success",
                          title: "Ditambahkan ke antrean",
                          message: item.hit.title,
                        });
                      }
                    }}
                    class="p-1 rounded-md text-neutral-400 hover:text-emerald-500 hover:bg-neutral-200/60 dark:hover:bg-white/10 transition-colors"
                  >
                    <Icon icon="lucide:list-end" class="w-4 h-4" />
                  </button>
                </div>
              {/if}

              <!-- Badges -->
              {#if item.kind === "command" && item.badge}
                <span
                  class="text-[10px] font-medium px-1.5 py-0.5 rounded shrink-0
                         {item.badgeVariant === 'green'
                           ? 'bg-emerald-500/15 text-emerald-600 dark:text-emerald-400 border border-emerald-500/20'
                           : item.badgeVariant === 'purple'
                           ? 'bg-purple-500/15 text-purple-600 dark:text-purple-400 border border-purple-500/20'
                           : 'bg-neutral-200/70 dark:bg-neutral-800 text-neutral-500 dark:text-neutral-400'}"
                >
                  {item.badge}
                </span>
              {:else if item.kind !== "command"}
                <span
                  class="text-[9.5px] uppercase tracking-wider font-semibold px-1.5 py-0.5 rounded
                         bg-neutral-100 dark:bg-white/5 text-neutral-400 dark:text-neutral-500 shrink-0"
                >
                  {item.category}
                </span>
              {/if}
            </div>
          {/each}
        {/if}
      </div>

      <!-- Footer Bar with Keyboard Hints -->
      <div
        class="flex items-center justify-between px-4 py-2 text-[11px] text-neutral-500 dark:text-neutral-400
               border-t border-neutral-200 dark:border-neutral-800/80 bg-neutral-50/70 dark:bg-[#0e1210]/70 select-none"
      >
        <div class="flex items-center gap-3 flex-wrap">
          <span class="inline-flex items-center gap-1">
            <kbd class="px-1.5 py-0.5 rounded bg-neutral-200 dark:bg-neutral-800 font-mono text-[10px]">↑↓</kbd> Navigasi
          </span>
          <span class="inline-flex items-center gap-1">
            <kbd class="px-1.5 py-0.5 rounded bg-neutral-200 dark:bg-neutral-800 font-mono text-[10px]">↵</kbd> Putar / Jalankan
          </span>
          <span class="inline-flex items-center gap-1">
            <kbd class="px-1.5 py-0.5 rounded bg-neutral-200 dark:bg-neutral-800 font-mono text-[10px]">⇧↵</kbd> Putar Berikutnya
          </span>
          <span class="inline-flex items-center gap-1">
            <kbd class="px-1.5 py-0.5 rounded bg-neutral-200 dark:bg-neutral-800 font-mono text-[10px]">⌃↵</kbd> Antrekan
          </span>
          <span class="inline-flex items-center gap-1">
            <kbd class="px-1.5 py-0.5 rounded bg-neutral-200 dark:bg-neutral-800 font-mono text-[10px]">&gt;</kbd> Perintah Cepat
          </span>
        </div>
        <span class="inline-flex items-center gap-1 shrink-0">
          <kbd class="px-1.5 py-0.5 rounded bg-neutral-200 dark:bg-neutral-800 font-mono text-[10px]">Esc</kbd> Tutup
        </span>
      </div>
    </div>
  </div>
{/if}
