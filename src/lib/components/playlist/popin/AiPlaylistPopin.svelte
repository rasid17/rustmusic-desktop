<script lang="ts">
  import { onMount, untrack } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { goto } from "$app/navigation";
  import Icon from "@iconify/svelte";
  import { t } from "#lib/i18n";
  import { errorMessage } from "#lib/helper/tools/errorTools";
  import { readLocal, writeLocal } from "#lib/helper/tools/storage";
  import { popinStore } from "#lib/stores/ui/popin.store";
  import { playlistStore } from "#lib/stores/playlist/playlist.store";
  import { profilSelector } from "#lib/stores/profil/profil.store";
  import { libraryStore } from "#lib/stores/library/library.store";
  import { toasts } from "#lib/stores/ui/toast.store";
  import { PLAYLIST_COLORS, PLAYLIST_ICONS } from "../playlistConfig";
  import FormField from "#lib/components/ui/input/FormField.svelte";
  import FormInput from "#lib/components/ui/input/FormInput.svelte";
  import PopinFooter from "#lib/components/ui/popin/PopinFooter.svelte";
  import StatusDot from "#lib/components/ui/status/StatusDot.svelte";
  import type { Playlist } from "#lib/types/db/playlist/Playlist";

  type AiAgentInfo = {
    id: string;
    name: string;
    description: string;
    available: boolean;
    binary_path: string | null;
    default_command: string;
  };

  type AiPlaylistResult = {
    playlist: Playlist;
    track_count: number;
    agent_used: string;
  };

  let {
    libraryId = null,
    initialPrompt = "",
    initialName = "",
  }: {
    libraryId?: number | null;
    initialPrompt?: string;
    initialName?: string;
  } = $props();

  let agents = $state<AiAgentInfo[]>([]);
  let loadingAgents = $state(true);
  let selectedAgentId = $state(readLocal("ai_playlist:agent", "claude"));
  let customCommand = $state(readLocal("ai_playlist:custom_cmd", ""));
  let prompt = $state(untrack(() => initialPrompt));
  let playlistName = $state(untrack(() => initialName));
  let maxTracks = $state<number>(Number(readLocal("ai_playlist:max_tracks", "25")) || 25);
  let selectedColorIndex = $state(4);
  let selectedIconIndex = $state(0);
  let isGenerating = $state(false);
  let errorText = $state<string | null>(null);

  const selectedColor = $derived(PLAYLIST_COLORS[selectedColorIndex] ?? "#8b5cf6");
  const selectedIcon = $derived(PLAYLIST_ICONS[selectedIconIndex] ?? PLAYLIST_ICONS[0]);
  const selectedAgent = $derived(agents.find((a) => a.id === selectedAgentId) ?? agents[0] ?? null);
  const submitStyle = $derived(`background: ${selectedColor}; color: #fff; box-shadow: 0 2px 12px ${selectedColor}40;`);

  const activeLibraryId = $derived(libraryId ?? $libraryStore.librarySelected?.id ?? null);

  const SUGGESTIONS = [
    "Pop & Indie santai tanpa musik orkestra/klasik",
    "Upbeat energetic tracks untuk fokus kerja",
    "Acoustic & warm vocals untuk malam hari",
    "Hidden gems yang jarang diputar tapi seirama",
  ];

  onMount(async () => {
    if (initialPrompt && !prompt) prompt = initialPrompt;
    if (initialName && !playlistName) playlistName = initialName;
    try {
      const list = await invoke<AiAgentInfo[]>("detect_ai_agents");
      agents = list;
      const saved = list.find((a) => a.id === selectedAgentId && a.available);
      const firstAvailable = list.find((a) => a.available && a.id !== "custom") ?? list[0];
      if (!saved && firstAvailable) {
        selectedAgentId = firstAvailable.id;
      }
    } catch (e) {
      console.error("Failed to detect AI agents:", e);
    } finally {
      loadingAgents = false;
    }
  });

  $effect(() => {
    if (selectedAgentId) writeLocal("ai_playlist:agent", selectedAgentId);
  });
  $effect(() => {
    writeLocal("ai_playlist:custom_cmd", customCommand);
  });
  $effect(() => {
    writeLocal("ai_playlist:max_tracks", String(maxTracks));
  });

  function close() {
    if (!isGenerating) popinStore.close();
  }

  async function submit() {
    errorText = null;
    if (!prompt.trim()) {
      errorText = "Masukkan deskripsi suasana, genre, atau instruksi playlist.";
      return;
    }
    const profilId = $profilSelector.profilSelected?.id;
    if (!profilId) {
      errorText = "Profil aktif tidak ditemukan.";
      return;
    }

    isGenerating = true;
    try {
      const res = await invoke<AiPlaylistResult>("generate_ai_playlist", {
        payload: {
          profil_id: profilId,
          library_id: activeLibraryId,
          agent_id: selectedAgentId,
          custom_command: customCommand.trim() || null,
          prompt: prompt.trim(),
          max_tracks: maxTracks,
          playlist_name: playlistName.trim() || null,
          color: selectedColor,
          icon: selectedIcon.id,
        },
      });

      await playlistStore.refresh();
      toasts.push({
        type: "success",
        title: res.playlist.name,
        message: `${res.track_count} lagu dipilih oleh ${res.agent_used}`,
      });
      popinStore.close();
      goto(`/playlist/${res.playlist.id}`);
    } catch (err) {
      errorText = errorMessage(err);
    } finally {
      isGenerating = false;
    }
  }
</script>

<div class="space-y-5">
  <!-- Aperçu de la playlist IA -->
  <div class="flex items-center gap-4 p-3.5 rounded-2xl border transition-colors"
       style="border-color: color-mix(in oklab, {selectedColor} 30%, transparent); background: linear-gradient(135deg, color-mix(in oklab, {selectedColor} 12%, transparent), transparent);">
    <span class="w-12 h-12 shrink-0 rounded-xl flex items-center justify-center transition-colors"
          style="background: color-mix(in oklab, {selectedColor} 16%, transparent); border: 1px solid color-mix(in oklab, {selectedColor} 30%, transparent); color: {selectedColor};">
      <Icon icon={selectedIcon.id} width="24" />
    </span>
    <div class="min-w-0 flex-1">
      <p class="text-[15px] font-bold truncate text-(--rg-tx)">
        {playlistName.trim() || "AI Curated Playlist"}
      </p>
      <p class="mt-0.5 text-xs text-(--rg-mu) truncate">
        {selectedAgent ? selectedAgent.name : "CLI Agent"} · {maxTracks} tracks max
      </p>
    </div>
  </div>

  <!-- Choix de l'agent IA -->
  <FormField label="AI Agent (CLI)" for="ai_agent">
    {#if loadingAgents}
      <div class="flex items-center gap-2 text-xs text-(--rg-mu) py-1">
        <Icon icon="lucide:loader-2" width="15" class="animate-spin" />
        Mendeteksi CLI agent di sistem...
      </div>
    {:else}
      <div id="ai_agent" class="flex flex-wrap gap-2">
        {#each agents as agent (agent.id)}
          {@const actif = selectedAgentId === agent.id}
          <button
            type="button"
            aria-pressed={actif}
            disabled={isGenerating || (!agent.available && agent.id !== "custom")}
            title={agent.binary_path ? `${agent.description} (${agent.binary_path})` : agent.description}
            class="h-9 px-3 inline-flex items-center gap-2 rounded-xl border text-[13px] font-semibold cursor-pointer transition-colors
                   disabled:opacity-40 disabled:cursor-not-allowed
                   {actif
                     ? 'border-(--rg-g)/60 bg-(--rg-gbg) text-(--rg-gtx)'
                     : 'border-(--rg-bd) text-(--rg-tx2) hover:bg-(--rg-hover) hover:text-(--rg-tx)'}"
            onclick={() => (selectedAgentId = agent.id)}
          >
            <Icon icon="material-symbols:terminal-rounded" width="16" />
            <span>{agent.name}</span>
            {#if agent.id !== "custom"}
              <StatusDot online={agent.available} />
            {/if}
          </button>
        {/each}
      </div>
    {/if}
  </FormField>

  {#if selectedAgentId === "custom" || selectedAgentId === "ollama"}
    <FormField
      label={selectedAgentId === "ollama" ? "Model Ollama" : "Perintah CLI Custom (menerima prompt via stdin)"}
      for="ai_custom_cmd"
    >
      <FormInput
        id="ai_custom_cmd"
        bind:value={customCommand}
        placeholder={selectedAgentId === "ollama" ? "llama3.2" : "agy -p / claude -p / gemini -p"}
        disabled={isGenerating}
      />
    </FormField>
  {/if}

  <!-- Prompt utilisateur -->
  <FormField
    label="Instruksi / Suasana Playlist"
    for="ai_prompt"
    required
    error={errorText ?? ""}
  >
    <FormInput
      id="ai_prompt"
      multiline
      bind:value={prompt}
      placeholder="Contoh: Lagu pop & synthpop santai untuk malam hari, jangan masukkan lagu orchestra atau klasik..."
      invalid={!!errorText}
      disabled={isGenerating}
      oninput={() => (errorText = null)}
    />
    <div class="mt-2 flex flex-wrap gap-1.5">
      {#each SUGGESTIONS as suggestion}
        <button
          type="button"
          disabled={isGenerating}
          class="px-2.5 py-1 rounded-lg text-[11px] font-medium bg-(--rg-s2) text-(--rg-tx2) hover:text-(--rg-tx) hover:bg-(--rg-hover) transition-colors cursor-pointer"
          onclick={() => {
            prompt = suggestion;
            errorText = null;
          }}
        >
          {suggestion}
        </button>
      {/each}
    </div>
  </FormField>

  <!-- Nombre de titres & Nom optionnel -->
  <div class="grid grid-cols-1 sm:grid-cols-[1fr_auto] gap-4 items-start">
    <FormField label={$t("forms.name")} for="ai_pl_name" optional={$t("forms.optional")}>
      <FormInput
        id="ai_pl_name"
        bind:value={playlistName}
        placeholder="Biarkan kosong agar diberi nama oleh AI"
        disabled={isGenerating}
      />
    </FormField>

    <FormField label="Jumlah Lagu" for="ai_max_tracks">
      <div id="ai_max_tracks" class="flex items-center gap-1.5 h-10">
        {#each [15, 25, 40, 60] as count}
          <button
            type="button"
            disabled={isGenerating}
            class="h-9 px-3 rounded-xl border text-xs font-bold cursor-pointer transition-colors
                   {maxTracks === count
                     ? 'border-(--rg-g)/60 bg-(--rg-gbg) text-(--rg-gtx)'
                     : 'border-(--rg-bd) text-(--rg-tx2) hover:bg-(--rg-hover)'}"
            onclick={() => (maxTracks = count)}
          >
            {count}
          </button>
        {/each}
      </div>
    </FormField>
  </div>

  <!-- Couleur & Icône -->
  <div class="grid grid-cols-[auto_1fr] items-center gap-x-5 gap-y-3">
    <span class="text-[11px] font-bold uppercase tracking-[0.1em] text-(--rg-mu)">{$t("forms.color")}</span>
    <div class="flex flex-wrap gap-2">
      {#each PLAYLIST_COLORS as color, i}
        <button
          type="button"
          aria-label={color}
          aria-pressed={selectedColorIndex === i}
          disabled={isGenerating}
          class="w-6 h-6 rounded-full cursor-pointer transition-transform hover:scale-110
                 {selectedColorIndex === i ? 'ring-2 ring-offset-2 ring-(--rg-tx) ring-offset-(--rg-carte) scale-110' : ''}"
          style="background: {color}"
          onclick={() => (selectedColorIndex = i)}
        ></button>
      {/each}
    </div>
    <span class="text-[11px] font-bold uppercase tracking-[0.1em] text-(--rg-mu)">{$t("forms.icon")}</span>
    <div class="flex flex-wrap gap-1">
      {#each PLAYLIST_ICONS as icon, i}
        <button
          type="button"
          aria-label={icon.id}
          aria-pressed={selectedIconIndex === i}
          disabled={isGenerating}
          class="w-8 h-8 flex items-center justify-center rounded-xl cursor-pointer transition-colors
                 {selectedIconIndex === i ? 'text-white' : 'text-(--rg-mu) hover:bg-(--rg-hover) hover:text-(--rg-tx)'}"
          style={selectedIconIndex === i ? `background: ${selectedColor}` : ""}
          onclick={() => (selectedIconIndex = i)}
        >
          <Icon icon={icon.id} width="17" />
        </button>
      {/each}
    </div>
  </div>
</div>

<PopinFooter
  bleed
  cancelLabel={$t("common.cancel")}
  oncancel={close}
  cancelDisabled={isGenerating}
  submitLabel={isGenerating ? "AI sedang menyusun..." : "Generate Playlist"}
  submitIcon="material-symbols:auto-awesome-rounded"
  onsubmit={submit}
  busy={isGenerating}
  {submitStyle}
/>
