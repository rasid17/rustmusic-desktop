<script lang="ts">
  // Racine de la fenêtre mini : une télécommande affichée. La lecture reste
  // pilotée par la fenêtre principale (voir `miniBridge`) ; cette fenêtre ne
  // fait qu'afficher l'état reçu et renvoyer les actions de l'utilisateur.
  import { onMount } from "svelte";
  import { get } from "svelte/store";
  import { invoke } from "@tauri-apps/api/core";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import MiniPlayer from "./MiniPlayer.svelte";
  import MicroPlayer from "./MicroPlayer.svelte";
  import { microPlayerActive } from "#lib/stores/ui/miniWindow.store";
  import { settingsStore } from "#lib/stores/settings/settings.store";
  import { initPlaybackPipelineListener } from "#lib/stores/player/playbackPipeline.store";
  import { initMiniClient, notifyMainReady, sendCmd } from "#lib/services/mini/miniBridge";
  import { initVolumeSync, loadVolume, volume, applyVolume } from "#lib/stores/player/volume.store";
  import { player } from "#lib/stores/player/player.store";
  import { applyAccent } from "#lib/helper/theme/accent";
  import { resolveShortcuts, eventToBinding, findAction, type ShortcutAction } from "#lib/config/shortcuts";

  // Couleur d'accent choisie dans Apparence (vide = vert d'origine).
  $effect(() => applyAccent($settingsStore.accent_color));

  // Raccourcis : les mêmes touches que la fenêtre principale, bornées à la
  // lecture et au volume — la fenêtre mini n'a ni recherche ni navigation.
  const shortcuts = $derived(resolveShortcuts($settingsStore.shortcuts));

  const shortcutHandlers: Record<ShortcutAction, () => void> = {
    playPause: () => sendCmd("toggle-play"),
    seekForward: () => sendCmd("seek", { position: (get(player).jsPosition ?? 0) + 10 }),
    seekBackward: () => sendCmd("seek", { position: Math.max(0, (get(player).jsPosition ?? 0) - 10) }),
    next: () => sendCmd("next"),
    previous: () => sendCmd("prev"),
    volumeUp: () => applyVolume(get(volume) + 5),
    volumeDown: () => applyVolume(get(volume) - 5),
    mute: () => { invoke('mute').then(loadVolume); },
    search: () => {},
    toggleMini: () => sendCmd("exit-mini"),
  };

  function handleKeydown(e: KeyboardEvent) {
    const target = e.target as HTMLElement;
    const tag = target?.tagName;
    if (tag === 'INPUT' || tag === 'TEXTAREA' || tag === 'SELECT' || target?.getAttribute?.('role') === 'slider') return;

    const binding = eventToBinding(e);
    const action = binding ? findAction(shortcuts, binding) : undefined;
    if (!action) return;
    e.preventDefault();
    shortcutHandlers[action]();
  }

  onMount(async () => {
    // L'écran de chargement d'app.html n'a pas lieu d'être : le lecteur est déjà là.
    document.getElementById("demarrage")?.remove();

    await settingsStore.init();
    initVolumeSync();
    void loadVolume();
    initPlaybackPipelineListener();
    const client = initMiniClient();

    // Attendre le premier état évite d'afficher une fenêtre vide ; le pont a
    // son propre filet (800 ms) si la fenêtre principale ne répond pas.
    await client.firstState;
    const win = getCurrentWindow();
    await win.show();
    await win.setFocus();
    // La fenêtre principale attend ce signal pour se masquer.
    notifyMainReady();

    // GTK3 n'honore les tailles demandées au widget que si la fenêtre a été
    // rendue non redimensionnable en cours de session (comme la principale en
    // entrant en mode mini) : figée une fois affichée, sans bords saisissables.
    await getCurrentWindow().setResizable(false);
  });
</script>

<svelte:window onkeydown={handleKeydown} />

{#if $microPlayerActive}
  <MicroPlayer />
{:else}
  <MiniPlayer />
{/if}
