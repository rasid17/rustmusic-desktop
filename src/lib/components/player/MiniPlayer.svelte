<script lang="ts">
  // Mini-lecteur multi-styles :
  // - 3 styles commutables : `classic` (Card / Bar / Large), `pill` (pochette ronde tournante), `sleeve` (CD optique 3 couches + égaliseur).
  // - Contrôles en verre liquide (« Liquid Glass »), temps de lecture affichés au survol, molette pour le volume, et tiroir inférieur (File / Paroles).
  import { onDestroy } from "svelte";
  import Icon from "@iconify/svelte";
  import { fade } from "svelte/transition";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { listen } from "@tauri-apps/api/event";
  import { invoke } from "@tauri-apps/api/core";
  import { detectOS } from "#lib/helper/tools/osDetection";
  import { subscribeCoverFallback } from "#lib/helper/tools/coverHelper";
  import LoadingRing from "#lib/components/ui/loader/LoadingRing.svelte";
  import CoverImg from "#lib/components/ui/image/CoverImg.svelte";
  import PlayerProgressBar from "./PlayerProgressBar.svelte";
  import CompactDiscArt from "./CompactDiscArt.svelte";
  import EqVisualizer from "./EqVisualizer.svelte";
  import { SpinClock } from "#lib/helper/player/spinClock";
  import { volume, applyVolume, loadVolume } from "#lib/stores/player/volume.store";
  import { t, currentLocale } from "#lib/i18n";
  import { player } from "#lib/stores/player/player.store";
  import { queueState } from "#lib/stores/queue/queueState.store";
  import { liked } from "#lib/stores/playlist/like.store";
  import { couleurMorceau } from "#lib/stores/player/trackColor.store";
  import { playbackPipelineStore, pipelineMode } from "#lib/stores/player/playbackPipeline.store";
  import { CHAINE } from "#lib/helper/audio/audioChain";
  import { displayTitle } from "#lib/helper/tools/stringTools";
  import { minutesSeconds } from "#lib/helper/tools/dateTools";
  import { dsdLabel, formatDsdRate, isDsdFormat } from "#lib/helper/tools/audioFormatTools";
  import { getLyrics, type Lyrics } from "#lib/services/lyrics/lyrics.service";
  import { parseLrc, findActiveLineIndex, type LrcLine } from "#lib/helper/lyrics/lrcParser";
  import {
    enterMicroPlayer,
    setMiniExpanded,
    reportCollapsedHeight,
    miniPinned,
    toggleMiniPin,
    miniPlayerStyle,
    miniClassicLayout,
    setMiniStyle,
    setMiniClassicLayout,
    cycleMiniStyle,
    type MiniPlayerStyle,
    type MiniClassicLayout,
  } from "#lib/stores/ui/miniWindow.store";
  import { sendCmd } from "#lib/services/mini/miniBridge";

  const SANS_POCHETTE = "/images/no-cd.png";

  const audioFile = $derived($player?.audioFile);
  const audioTags = $derived(audioFile?.tags);
  const episode = $derived(audioTags?.media_type === "podcast");
  const pathFile = $derived($player?.pathFile ?? null);
  const hasTrack = $derived(!!pathFile);
  const isPlaying = $derived($player?.status === "playing");
  const enAttente = $derived(isPlaying && !!$player?.isPreparing);
  const trackTitle = $derived(displayTitle(audioTags?.title, pathFile, $t("common.unknown_title")));
  const artistName = $derived(audioTags?.artist || $t("common.unknown_artist"));
  const attachedCover = $derived(audioTags?.attached_images?.[0]?.image_src ?? null);
  const currentQueueCover = $derived($queueState.tracks[$queueState.currentIndex]?.cover ?? null);
  let fallbackCover = $state<string | null>(null);
  $effect(() => {
    if (!hasTrack || attachedCover) {
      fallbackCover = null;
      return;
    }
    return subscribeCoverFallback(currentQueueCover, "1x", (url) => {
      fallbackCover = url;
    });
  });
  const coverSrc = $derived(attachedCover || fallbackCover);
  const duration = $derived($player?.duration ?? 0);
  const jsPosition = $derived($player?.jsPosition ?? 0);
  const remaining = $derived(Math.max(0, duration - jsPosition));
  const percent = $derived(duration ? Math.min(100, Math.max(0, (jsPosition / duration) * 100)) : 0);
  const aime = $derived(!!pathFile && $liked.paths.has(pathFile));
  const isShuffled = $derived(!!$queueState.isShuffled);
  const repeatMode = $derived($queueState.repeatMode ?? "off");

  // ─── Qualité audio (« FLAC · 24/96 · Bit-perfect ») ───
  const nombre = $derived(new Intl.NumberFormat($currentLocale, { maximumFractionDigits: 1 }));
  const dsd = $derived(isDsdFormat(audioFile?.audio_format));
  const format = $derived(!audioFile ? null : dsd ? dsdLabel(audioFile.sample_rate) : audioFile.audio_format?.toUpperCase() ?? null);
  const qualite = $derived.by(() => {
    const sr = audioFile?.sample_rate;
    if (!sr) return "";
    if (dsd) return formatDsdRate(sr);
    const khz = nombre.format(sr / 1000);
    return audioFile?.bits_per_sample ? `${audioFile.bits_per_sample}/${khz}` : `${khz} kHz`;
  });
  const chaine = $derived.by(() => {
    const m = pipelineMode($playbackPipelineStore, $volume);
    return m ? CHAINE[m] : null;
  });

  // ─── Survol de la fenêtre (temps de lecture affichés au survol) ───
  const gtkHover = detectOS() === "linux";
  let pointerInside = $state(false);
  let lastPointer = { x: NaN, y: NaN };
  let ignoreMoveUntil = 0;

  function setPointerInside(inside: boolean) {
    pointerInside = inside;
    if (!inside) {
      lastPointer = { x: NaN, y: NaN };
      ignoreMoveUntil = performance.now() + 120;
    }
  }

  $effect(() => {
    if (!gtkHover) return;
    const off = listen<boolean>("window-pointer", (e) => setPointerInside(e.payload));
    const onDocLeave = () => setPointerInside(false);
    const onWinBlur = () => setPointerInside(false);
    document.documentElement.addEventListener("mouseleave", onDocLeave);
    window.addEventListener("blur", onWinBlur);
    return () => {
      void off.then((f) => f());
      document.documentElement.removeEventListener("mouseleave", onDocLeave);
      window.removeEventListener("blur", onWinBlur);
    };
  });

  function onPointerMove(e: PointerEvent) {
    if (!gtkHover) {
      pointerInside = true;
      return;
    }
    if (performance.now() < ignoreMoveUntil) return;
    if (e.clientX <= 1 || e.clientY <= 1 || e.clientX >= window.innerWidth - 1 || e.clientY >= window.innerHeight - 1) {
      return;
    }
    if (Number.isNaN(lastPointer.x)) {
      lastPointer = { x: e.clientX, y: e.clientY };
      return;
    }
    if (e.clientX === lastPointer.x && e.clientY === lastPointer.y) return;
    lastPointer = { x: e.clientX, y: e.clientY };
    pointerInside = true;
  }

  function onPointerEnter(e: PointerEvent) {
    if (!gtkHover) {
      pointerInside = true;
      return;
    }
    if (performance.now() < ignoreMoveUntil) return;
    lastPointer = { x: e.clientX, y: e.clientY };
    pointerInside = true;
  }

  function onPointerLeave() {
    setPointerInside(false);
  }

  const timeVisibleClass = $derived(pointerInside ? "opacity-100" : "opacity-0");

  // ─── Rotation inertielle (SpinClock) pour la pochette ronde du mode Pill ───
  let pillAngle = $state(0);
  const pillSpinner = new SpinClock((a) => {
    pillAngle = a;
  });

  $effect(() => {
    pillSpinner.setRunning($miniPlayerStyle === "pill" && isPlaying);
  });

  onDestroy(() => {
    pillSpinner.destroy();
    if (hintTimer) clearTimeout(hintTimer);
  });

  // ─── Molette de souris -> Volume (±5 %) + bulle d'indication ───
  const dopActive = $derived($playbackPipelineStore?.backend?.endsWith("DoP") === true);
  const muted = $derived($volume === 0);
  const volumeIcon = $derived(
    muted
      ? "material-symbols:volume-off-rounded"
      : $volume < 50
        ? "material-symbols:volume-down-rounded"
        : "material-symbols:volume-up-rounded"
  );

  let volumeHint = $state(false);
  let hintTimer: ReturnType<typeof setTimeout> | null = null;

  function onWheel(e: WheelEvent) {
    if ((e.target as HTMLElement | null)?.closest("[data-scrollable]")) return;
    if (dopActive) return;
    applyVolume($volume + (e.deltaY < 0 ? 5 : -5));
    volumeHint = true;
    if (hintTimer) clearTimeout(hintTimer);
    hintTimer = setTimeout(() => (volumeHint = false), 950);
  }

  function toggleMute() {
    if (dopActive) return;
    invoke("mute").then(loadVolume).catch(() => {});
  }

  // ─── Déplacement de la fenêtre par glisser sur toute zone libre ───
  function onPointerDown(e: PointerEvent) {
    if (
      e.button !== 0 ||
      (e.target as HTMLElement).closest("button, input, [role=slider], [data-no-drag]")
    ) {
      return;
    }
    void getCurrentWindow().startDragging();
  }

  // ─── Menu « … » (sélecteur de style Classic | Pill | Sleeve + actions) ───
  let menuOpen = $state(false);

  function selectStyle(s: MiniPlayerStyle) {
    void setMiniStyle(s);
  }

  function selectClassicLayout(l: MiniClassicLayout) {
    void setMiniClassicLayout(l);
  }

  // ─── Tiroir inférieur (File d'attente / Paroles) ───
  type Onglet = "queue" | "lyrics" | null;
  let onglet = $state<Onglet>(null);
  function toggleTab(o: Exclude<Onglet, null>) {
    onglet = onglet === o ? null : o;
    menuOpen = false;
    setMiniExpanded(onglet !== null);
  }

  // ─── File : le titre en cours en tête, puis la suite ───
  const currentIndex = $derived($queueState.currentIndex);
  const enCours = $derived(currentIndex > -1 ? ($queueState.tracks[currentIndex] ?? null) : null);
  const suite = $derived(currentIndex > -1 ? $queueState.tracks.slice(currentIndex + 1) : $queueState.tracks);
  const aVenir = $derived(suite.length + (enCours ? 1 : 0));

  const absoluteIndex = (i: number) => (currentIndex > -1 ? currentIndex + 1 : 0) + i;

  function playAt(i: number) {
    sendCmd("play-at", { index: absoluteIndex(i) });
  }

  let depart = $state<number | null>(null);
  let survol = $state<number | null>(null);
  function grab(e: PointerEvent, i: number) {
    if (e.button !== 0) return;
    e.preventDefault();
    e.stopPropagation();
    depart = i;
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
  }
  function drag(e: PointerEvent) {
    if (depart === null) return;
    const ligne = document.elementFromPoint(e.clientX, e.clientY)?.closest("[data-qi]") as HTMLElement | null;
    if (ligne) survol = Number(ligne.dataset.qi);
  }
  function drop() {
    if (depart !== null && survol !== null && depart !== survol) {
      sendCmd("reorder", { from: absoluteIndex(depart), to: absoluteIndex(survol) });
    }
    depart = null;
    survol = null;
  }

  // ─── Paroles synchronisées ───
  let lignes = $state<LrcLine[]>([]);
  let texteBrut = $state<string | null>(null);
  let source = $state<Lyrics["source"] | null>(null);
  let etat = $state<"idle" | "loading" | "ready" | "empty">("idle");
  let parolesDe = "";
  let boite = $state<HTMLDivElement | null>(null);
  const elLignes: Record<number, HTMLElement> = {};

  const active = $derived(lignes.length > 0 ? findActiveLineIndex(lignes, jsPosition * 1000) : -1);

  async function loadLyrics(path: string) {
    etat = "loading";
    lignes = [];
    texteBrut = null;
    source = null;
    parolesDe = path;
    try {
      const res = await getLyrics(path);
      if (parolesDe !== path) return;
      if (!res || (!res.synced && !res.plain)) {
        etat = "empty";
        return;
      }
      if (res.synced) lignes = parseLrc(res.synced);
      texteBrut = res.plain;
      source = res.source;
      etat = lignes.length > 0 || res.plain ? "ready" : "empty";
    } catch {
      etat = "empty";
    }
  }

  $effect(() => {
    if (onglet === "lyrics" && pathFile && pathFile !== parolesDe) loadLyrics(pathFile);
  });

  let dejaCentre = false;
  $effect(() => {
    const i = active;
    if (onglet !== "lyrics" || i < 0 || !boite) {
      dejaCentre = false;
      return;
    }
    const el = elLignes[i];
    if (!el) return;
    const haut = el.offsetTop - boite.clientHeight / 2 + el.offsetHeight / 2;
    boite.scrollTo({ top: haut, behavior: dejaCentre ? "smooth" : "instant" });
    dejaCentre = true;
  });

  const SOURCES: Record<string, string> = {
    sidecar: "mini.src_sidecar",
    lrclib: "mini.src_lrclib",
    lyricsovh: "mini.src_ovh",
    manual: "mini.src_manual",
    server: "mini.src_server",
  };

  // ─── Mesure dynamique de la hauteur repliée ───
  let entete = $state<HTMLDivElement | null>(null);
  $effect(() => {
    const el = entete;
    const _style = $miniPlayerStyle;
    const _layout = $miniClassicLayout;
    if (!el) return;
    reportCollapsedHeight(el.offsetHeight);
    const ro = new ResizeObserver(() => reportCollapsedHeight(el.offsetHeight));
    ro.observe(el);
    return () => ro.disconnect();
  });

  function fermer() {
    sendCmd("close-app");
  }

  const entetePanneau =
    "flex items-center justify-between px-4 pt-3 pb-1.5 text-[10px] font-bold uppercase tracking-[0.12em] text-(--lc-mu)";
</script>

{#snippet vignette(path: string | null | undefined)}
  <span class="w-10 h-10 shrink-0 rounded-md overflow-hidden bg-(--lc-survol)">
    {#if path && path !== SANS_POCHETTE}
      <CoverImg {path} alt="" size="1x" class="w-full h-full object-cover" />
    {:else}
      <img src={SANS_POCHETTE} alt="" class="w-full h-full object-cover" />
    {/if}
  </span>
{/snippet}

{#snippet transportRow(compact = false, onDark = false)}
  {@const circleCls = onDark
    ? "bg-white/16 hover:bg-white/26 active:bg-white/34 text-white border border-white/15"
    : "mini-glass-circle text-(--lc-ic-fort)"}
  {@const glyphCls = onDark
    ? "text-white hover:bg-white/16 active:scale-94"
    : "text-(--lc-ic-fort) hover:bg-(--lc-survol) active:scale-94"}
  {@const sideSize = compact ? "w-6.5 h-6.5" : "w-7.5 h-7.5"}
  {@const skipSize = compact ? "w-7.5 h-7.5" : "w-9 h-9"}
  {@const playSize = compact ? "w-8.5 h-8.5" : "w-10.5 h-10.5"}

  <div class="flex items-center justify-between gap-2 w-full">
    <button
      type="button"
      class="{sideSize} shrink-0 flex items-center justify-center rounded-full cursor-pointer transition-all {circleCls}
             {isShuffled ? 'text-(--lc-acc)! ring-1 ring-(--lc-acc)/45' : 'opacity-80 hover:opacity-100'}"
      title={$t("player.shuffle")}
      aria-label={$t("player.shuffle")}
      aria-pressed={isShuffled}
      onclick={() => sendCmd("toggle-shuffle")}
    >
      <Icon icon="material-symbols:shuffle-rounded" width={compact ? 15 : 17} />
    </button>

    <div class="flex items-center gap-1.5">
      <button
        type="button"
        class="{skipSize} flex items-center justify-center rounded-full cursor-pointer transition-all {glyphCls}"
        title={$t("player.prev")}
        aria-label={$t("player.prev")}
        onclick={() => sendCmd("prev")}
      >
        <Icon icon="material-symbols:skip-previous-rounded" width={compact ? 22 : 26} />
      </button>

      <button
        type="button"
        class="lecteur-lecture relative {playSize} flex items-center justify-center rounded-full cursor-pointer transition-transform hover:scale-105 active:scale-95
               {onDark ? 'bg-white text-black shadow-[0_6px_20px_rgba(0,0,0,0.45)]' : 'bg-(--lc-play) text-(--lc-play-tx)'}"
        title={enAttente ? $t("player.preparing") : isPlaying ? $t("player.pause") : $t("player.play")}
        aria-label={isPlaying ? $t("player.pause") : $t("player.play")}
        aria-busy={enAttente}
        onclick={() => sendCmd("toggle-play")}
      >
        <Icon icon={isPlaying ? "material-symbols:pause-rounded" : "material-symbols:play-arrow-rounded"} width={compact ? 22 : 26} />
        <LoadingRing actif={enAttente} />
      </button>

      <button
        type="button"
        class="{skipSize} flex items-center justify-center rounded-full cursor-pointer transition-all {glyphCls}"
        title={$t("player.next")}
        aria-label={$t("player.next")}
        onclick={() => sendCmd("next")}
      >
        <Icon icon="material-symbols:skip-next-rounded" width={compact ? 22 : 26} />
      </button>
    </div>

    <button
      type="button"
      class="{sideSize} shrink-0 flex items-center justify-center rounded-full cursor-pointer transition-all {circleCls}
             {repeatMode !== 'off' ? 'text-(--lc-acc)! ring-1 ring-(--lc-acc)/45' : 'opacity-80 hover:opacity-100'}"
      title={repeatMode === "one" ? $t("player.repeat_one") : repeatMode === "all" ? $t("player.repeat_all") : $t("player.repeat_off")}
      aria-label={$t("player.repeat_all")}
      aria-pressed={repeatMode !== "off"}
      onclick={() => sendCmd("cycle-repeat")}
    >
      <Icon
        icon={repeatMode === "one" ? "material-symbols:repeat-one-rounded" : "material-symbols:repeat-rounded"}
        width={compact ? 15 : 17}
      />
    </button>
  </div>
{/snippet}

{#snippet topActions(compact = false, onDark = false)}
  {@const btnSize = compact ? "w-6 h-6" : "w-7 h-7"}
  {@const baseCls = onDark
    ? "bg-black/38 hover:bg-black/55 backdrop-blur-md text-white border border-white/18"
    : "mini-glass-circle text-(--lc-ic-fort)"}
  <div class="flex items-center gap-1 shrink-0">
    {#if hasTrack && !episode}
      <button
        type="button"
        class="favori {btnSize} flex items-center justify-center rounded-full cursor-pointer {baseCls}
               {aime ? 'text-[#ff5d73]!' : ''}"
        title={$t("player.like")}
        aria-label={$t("player.like")}
        aria-pressed={aime}
        onclick={() => pathFile && sendCmd("toggle-like", { path: pathFile })}
      >
        <Icon icon={aime ? "material-symbols:favorite-rounded" : "material-symbols-light:favorite-outline-rounded"} width={compact ? 14 : 16} />
      </button>
    {/if}
    <button
      type="button"
      class="{btnSize} flex items-center justify-center rounded-full cursor-pointer {baseCls}"
      title={$t("mini.style_switch").replace("{style}", $t($miniPlayerStyle === "pill" ? "mini.style_pill" : $miniPlayerStyle === "sleeve" ? "mini.style_sleeve" : "mini.style_classic"))}
      aria-label={$t("mini.style_switch_aria")}
      onclick={() => void cycleMiniStyle()}
    >
      <Icon
        icon={$miniPlayerStyle === "pill"
          ? "ph:vinyl-record-duotone"
          : $miniPlayerStyle === "sleeve"
            ? "ph:disc-duotone"
            : "ph:cards-duotone"}
        width={compact ? 14 : 16}
      />
    </button>
    <button
      type="button"
      class="{btnSize} flex items-center justify-center rounded-full cursor-pointer {baseCls}
             {menuOpen ? 'ring-1 ring-(--lc-acc)' : ''}"
      title={$t("common.menu")}
      aria-label={$t("common.menu")}
      aria-expanded={menuOpen}
      onclick={() => (menuOpen = !menuOpen)}
    >
      <Icon icon="material-symbols:more-horiz" width={compact ? 16 : 18} />
    </button>
  </div>
{/snippet}

<!-- Conteneur racine de la fenêtre mini -->
<div
  class="lecteur fixed inset-0 flex flex-col overflow-hidden select-none cursor-grab active:cursor-grabbing
         {$miniPlayerStyle === 'pill' && !onglet ? 'bg-transparent! shadow-none!' : 'mini-lecteur rounded-[24px]'}"
  style:--lh={$couleurMorceau.h}
  style:--ls={$couleurMorceau.s}
  role="presentation"
  onpointerdown={onPointerDown}
  onpointerenter={onPointerEnter}
  onpointermove={onPointerMove}
  onpointerleave={onPointerLeave}
  onmouseleave={onPointerLeave}
  onwheel={onWheel}
>
  <!-- ═══════════════════════════════════════════════════════════════════════
       PARTIE REPLIÉE (mesurée par ResizeObserver)
       ═══════════════════════════════════════════════════════════════════════ -->
  <div bind:this={entete} class="relative shrink-0">
    <!-- Liseré spéculaire supérieur (Liquid Glass Top Specular Hairline) -->
    {#if $miniPlayerStyle !== "pill"}
      <div
        class="pointer-events-none absolute inset-x-7 top-0 h-px z-30 opacity-75"
        style:background="var(--mini-specular)"
      ></div>
    {/if}

    <!-- ─────────────────────────────────────────────────────────────────────
         DESIGN 1 : PILL (Poin 6 — Pochette ronde 120×120 tournante à gauche + capsule à droite)
         ───────────────────────────────────────────────────────────────────── -->
    {#if $miniPlayerStyle === "pill"}
      <div class="relative flex items-center min-h-[132px] px-1.5 py-1.5">
        <!-- Carte capsule à droite (démarre à x=60, exactement sous le centre du disque 120×120) -->
        <div
          class="mini-lecteur relative w-full ml-[58px] min-h-[118px] rounded-l-none rounded-r-[26px] pl-[70px] pr-3.5 py-2.5 flex flex-col justify-between overflow-hidden"
        >
          <div
            class="pointer-events-none absolute left-14 right-6 top-0 h-px opacity-75"
            style:background="var(--mini-specular)"
          ></div>

          <!-- Ligne 1 : Titre + Artiste + Boutons (Cœur / Style / …) -->
          <div class="flex items-start justify-between gap-2 min-w-0">
            <div class="min-w-0 flex-1">
              <div class="truncate text-[13.5px] font-extrabold tracking-[-0.01em] text-(--lc-tx) leading-tight" title={trackTitle}>
                {hasTrack ? trackTitle : $t("player.no_track")}
              </div>
              <div class="truncate text-[11.5px] font-semibold text-(--lc-acc) leading-tight mt-0.5">
                {hasTrack ? artistName : "RustMusic"}
              </div>
            </div>
            {@render topActions(true, false)}
          </div>

          <!-- Ligne 2 : Temps (au survol) + Barre de progression -->
          <div class="flex items-center gap-2 my-1" data-no-drag>
            <span class="font-mono text-[9.5px] tabular-nums text-(--lc-mu) transition-opacity duration-180 {timeVisibleClass}">
              {minutesSeconds(jsPosition)}
            </span>
            <div class="flex-1 min-w-0">
              <PlayerProgressBar
                position={percent}
                onseek={(p) => duration && sendCmd("seek", { position: (p / 100) * duration })}
              />
            </div>
            <span class="font-mono text-[9.5px] tabular-nums text-(--lc-mu) transition-opacity duration-180 {timeVisibleClass}">
              -{minutesSeconds(remaining)}
            </span>
          </div>

          <!-- Ligne 3 : Transport 5 boutons -->
          {@render transportRow(true, false)}
        </div>

        <!-- Pochette ronde 120×120 à gauche (tourne avec inertie SpinClock) -->
        <div
          class="absolute left-1.5 top-1/2 -translate-y-1/2 w-[118px] h-[118px] rounded-full overflow-hidden z-10
                 shadow-[0_8px_24px_rgba(0,0,0,0.48)] ring-1 ring-white/25 bg-zinc-900 pointer-events-none"
        >
          <div
            class="w-full h-full rounded-full will-change-transform"
            style:transform="rotate({pillAngle}deg)"
          >
            <img
              src={hasTrack && coverSrc ? coverSrc : SANS_POCHETTE}
              alt=""
              class="w-full h-full object-cover"
              onerror={(e) => { const img = e.currentTarget as HTMLImageElement; if (!img.src.endsWith(SANS_POCHETTE)) img.src = SANS_POCHETTE; }}
            />
          </div>
          <!-- Reflet spéculaire fixe sur la pochette ronde + anneau central discret -->
          <div
            class="absolute inset-0 rounded-full bg-[linear-gradient(135deg,rgba(255,255,255,0.22)_0%,transparent_42%,transparent_65%,rgba(255,255,255,0.1)_100%)]"
          ></div>
          <div class="absolute inset-0 rounded-full ring-1 ring-inset ring-white/20"></div>
        </div>
      </div>

    <!-- ─────────────────────────────────────────────────────────────────────
         DESIGN 2 : SLEEVE (Poin 7 — CD optique 380×380 débordant du haut + EqVisualizer)
         ───────────────────────────────────────────────────────────────────── -->
    {:else if $miniPlayerStyle === "sleeve"}
      <div class="relative flex flex-col items-center px-4 pb-3.5 pt-0 overflow-hidden">
        <!-- Lueur d'ambiance issue de la pochette -->
        {#if hasTrack && coverSrc}
          <img
            src={coverSrc}
            alt=""
            class="pointer-events-none absolute inset-x-0 -top-12 w-full h-56 object-cover blur-3xl opacity-30 scale-125"
            onerror={(e) => { const img = e.currentTarget as HTMLImageElement; if (!img.src.endsWith(SANS_POCHETTE)) img.src = SANS_POCHETTE; }}
          />
        {/if}

        <!-- Actions rapides en haut à droite & gauche (flottantes sur les coins de la pochette) -->
        <div class="absolute top-3 inset-x-3.5 flex items-center justify-between z-20 pointer-events-none">
          <div class="flex items-center gap-1 pointer-events-auto">
            <button
              type="button"
              class="w-7 h-7 flex items-center justify-center rounded-full cursor-pointer mini-glass-circle
                     {$miniPinned ? 'text-(--lc-acc)!' : 'text-(--lc-ic-fort) opacity-75 hover:opacity-100'}"
              title={$t("mini.pin")}
              aria-label={$t("mini.pin")}
              aria-pressed={$miniPinned}
              onclick={toggleMiniPin}
            >
              <Icon icon={$miniPinned ? "material-symbols:keep" : "material-symbols-light:keep-outline"} width="15" />
            </button>
          </div>
          <div class="pointer-events-auto">
            {@render topActions(false, false)}
          </div>
        </div>

        <!-- Compact Disc réaliste 380×380 débordant du plafond (-mt-[200px] => ~180px visibles) -->
        <div class="-mt-[200px] z-10 flex justify-center">
          <CompactDiscArt
            src={hasTrack ? coverSrc : null}
            fallbackSrc={SANS_POCHETTE}
            spinning={isPlaying}
            size={380}
          />
        </div>

        <!-- Égaliseur 5 barres animé sous le CD -->
        <div class="mt-2.5 mb-1 flex items-center justify-center gap-2 z-10">
          <EqVisualizer active={isPlaying} />
          {#if hasTrack && format}
            <span class="px-1.5 py-0.5 rounded-[5px] font-mono text-[9.5px] font-medium tracking-[0.04em] bg-(--lc-fmt-bg) text-(--lc-fmt-tx)">
              {format}{#if qualite} · {qualite}{/if}
            </span>
          {/if}
        </div>

        <!-- Artiste (accent) + Titre centrés -->
        <div class="w-full text-center px-2 z-10">
          <div class="truncate text-[12.5px] font-semibold text-(--lc-acc)">
            {hasTrack ? artistName : "RustMusic"}
          </div>
          <div class="truncate text-[15.5px] font-extrabold tracking-[-0.01em] text-(--lc-tx) mt-0.5" title={trackTitle}>
            {hasTrack ? trackTitle : $t("player.no_track")}
          </div>
        </div>

        <!-- Barre de progression + temps au survol -->
        <div class="w-full mt-2.5 z-10" data-no-drag>
          <PlayerProgressBar
            position={percent}
            onseek={(p) => duration && sendCmd("seek", { position: (p / 100) * duration })}
          />
          <div class="flex justify-between font-mono text-[10px] tabular-nums text-(--lc-mu) -mt-0.5 transition-opacity duration-180 {timeVisibleClass}">
            <span>{minutesSeconds(jsPosition)}</span>
            <span>-{minutesSeconds(remaining)}</span>
          </div>
        </div>

        <!-- Transport 5 boutons -->
        <div class="w-full mt-1.5 px-2 z-10">
          {@render transportRow(false, false)}
        </div>
      </div>

    <!-- ─────────────────────────────────────────────────────────────────────
         DESIGN 3 : CLASSIC — Variante « BAR » (420×158, Live-Activity horizontal)
         ───────────────────────────────────────────────────────────────────── -->
    {:else if $miniClassicLayout === "bar"}
      <div class="flex flex-col gap-1.5 px-3.5 py-3">
        <!-- Ligne 1 : Pochette 56×56 + Titre/Artiste + Actions -->
        <div class="flex items-center gap-3 min-w-0">
          <div class="lecteur-pochette w-14 h-14 shrink-0 rounded-[13px] overflow-hidden">
            <img
              src={hasTrack && coverSrc ? coverSrc : SANS_POCHETTE}
              alt=""
              class="w-full h-full object-cover"
              onerror={(e) => ((e.currentTarget as HTMLImageElement).src = SANS_POCHETTE)}
            />
          </div>
          <div class="min-w-0 flex-1">
            <div class="truncate text-[14.5px] font-extrabold tracking-[-0.01em] text-(--lc-tx)" title={trackTitle}>
              {hasTrack ? trackTitle : $t("player.no_track")}
            </div>
            <div class="flex items-center gap-2 min-w-0 mt-0.5">
              <span class="truncate text-xs font-semibold text-(--lc-acc)">
                {hasTrack ? artistName : "RustMusic"}
              </span>
              {#if hasTrack && format}
                <span class="shrink-0 px-1.5 py-px rounded font-mono text-[9.5px] bg-(--lc-fmt-bg) text-(--lc-fmt-tx)">
                  {format}
                </span>
              {/if}
            </div>
          </div>
          {@render topActions(false, false)}
        </div>

        <!-- Ligne 2 : Temps (survol) + Progression -->
        <div class="flex items-center gap-2.5" data-no-drag>
          <span class="font-mono text-[10px] tabular-nums text-(--lc-mu) transition-opacity duration-180 {timeVisibleClass}">
            {minutesSeconds(jsPosition)}
          </span>
          <div class="flex-1 min-w-0">
            <PlayerProgressBar
              position={percent}
              onseek={(p) => duration && sendCmd("seek", { position: (p / 100) * duration })}
            />
          </div>
          <span class="font-mono text-[10px] tabular-nums text-(--lc-mu) transition-opacity duration-180 {timeVisibleClass}">
            -{minutesSeconds(remaining)}
          </span>
        </div>

        <!-- Ligne 3 : Transport 5 boutons -->
        <div class="px-2">
          {@render transportRow(true, false)}
        </div>
      </div>

    <!-- ─────────────────────────────────────────────────────────────────────
         DESIGN 3 : CLASSIC — Variante « LARGE » (340×420, Full-Bleed Cover + Frosted Band)
         ───────────────────────────────────────────────────────────────────── -->
    {:else if $miniClassicLayout === "large"}
      <div class="relative w-full h-[420px] flex flex-col justify-end overflow-hidden bg-black text-white">
        <!-- Image plein cadre (Full-Bleed Artwork) -->
        <img
          src={hasTrack && coverSrc ? coverSrc : SANS_POCHETTE}
          alt=""
          class="pointer-events-none absolute inset-0 w-full h-full object-cover"
          onerror={(e) => { const img = e.currentTarget as HTMLImageElement; if (!img.src.endsWith(SANS_POCHETTE)) img.src = SANS_POCHETTE; }}
        />

        <!-- Bande givrée dynamique au bas (MiniFrostBand : copie floutée + masque dégradé + voile de contraste) -->
        <div
          class="pointer-events-none absolute inset-x-0 bottom-0 h-[230px] overflow-hidden"
          style="-webkit-mask-image: linear-gradient(to bottom, transparent 0%, rgba(0,0,0,0.65) 24%, #000 46%); mask-image: linear-gradient(to bottom, transparent 0%, rgba(0,0,0,0.65) 24%, #000 46%);"
        >
          <img
            src={hasTrack && coverSrc ? coverSrc : SANS_POCHETTE}
            alt=""
            class="w-full h-[420px] object-cover object-bottom blur-2xl scale-115 brightness-75 saturate-140"
            onerror={(e) => { const img = e.currentTarget as HTMLImageElement; if (!img.src.endsWith(SANS_POCHETTE)) img.src = SANS_POCHETTE; }}
          />
          <div class="absolute inset-0 bg-gradient-to-t from-black/82 via-black/58 to-transparent"></div>
        </div>

        <!-- Bouton Pin en haut à gauche au survol -->
        <div class="absolute top-3 left-3.5 z-20 transition-opacity duration-180 {pointerInside || $miniPinned ? 'opacity-100' : 'opacity-0 pointer-events-none'}">
          <button
            type="button"
            class="w-7 h-7 flex items-center justify-center rounded-full cursor-pointer bg-black/40 hover:bg-black/60 backdrop-blur-md border border-white/18
                   {$miniPinned ? 'text-(--lc-acc)!' : 'text-white/85'}"
            title={$t("mini.pin")}
            aria-label={$t("mini.pin")}
            aria-pressed={$miniPinned}
            onclick={toggleMiniPin}
          >
            <Icon icon={$miniPinned ? "material-symbols:keep" : "material-symbols-light:keep-outline"} width="15" />
          </button>
        </div>

        <!-- Contenu inférieur sur la bande givrée -->
        <div class="relative z-10 flex flex-col gap-2 px-4 pb-3.5 pt-8">
          <div class="flex items-center justify-between gap-2 min-w-0">
            <div class="min-w-0 flex-1">
              <div class="truncate text-[16px] font-extrabold tracking-[-0.01em] text-white drop-shadow-sm" title={trackTitle}>
                {hasTrack ? trackTitle : $t("player.no_track")}
              </div>
              <div class="truncate text-[13px] font-semibold text-white/80">
                {hasTrack ? artistName : "RustMusic"}
              </div>
            </div>
            {@render topActions(false, true)}
          </div>

          <div data-no-drag>
            <PlayerProgressBar
              position={percent}
              onseek={(p) => duration && sendCmd("seek", { position: (p / 100) * duration })}
            />
            <div class="flex justify-between font-mono text-[10px] tabular-nums text-white/75 -mt-0.5 transition-opacity duration-180 {timeVisibleClass}">
              <span>{minutesSeconds(jsPosition)}</span>
              <span>-{minutesSeconds(remaining)}</span>
            </div>
          </div>

          {@render transportRow(false, true)}

          <!-- Barre de volume intégrée en bas -->
          {#if !dopActive}
            <div class="flex items-center gap-2 pt-1 opacity-85 hover:opacity-100 transition-opacity" data-no-drag>
              <button
                type="button"
                class="text-white/75 hover:text-white cursor-pointer"
                title={muted ? $t("player.unmute") : $t("player.mute")}
                onclick={toggleMute}
              >
                <Icon icon={volumeIcon} width="16" />
              </button>
              <input
                type="range"
                min="0"
                max="100"
                step="1"
                value={$volume}
                oninput={(e) => applyVolume(Number((e.currentTarget as HTMLInputElement).value))}
                class="flex-1 h-1 accent-white cursor-pointer"
                aria-label={$t("common.volume")}
              />
              <span class="w-7 text-right font-mono text-[10px] tabular-nums text-white/75">{$volume}%</span>
            </div>
          {/if}
        </div>
      </div>

    <!-- ─────────────────────────────────────────────────────────────────────
         DESIGN 3 : CLASSIC — Variante « CARD » (par défaut, 352×248)
         ───────────────────────────────────────────────────────────────────── -->
    {:else}
      <div class="flex flex-col gap-2.5 px-4 pt-3.5 pb-3.5">
        <!-- Ligne 1 : Pochette 72×72 + Titre + Artiste + Actions verre -->
        <div class="flex items-center gap-3.5 min-w-0">
          <div class="lecteur-pochette w-18 h-18 shrink-0 rounded-2xl overflow-hidden">
            <img
              src={hasTrack && coverSrc ? coverSrc : SANS_POCHETTE}
              alt=""
              class="w-full h-full object-cover"
              onerror={(e) => ((e.currentTarget as HTMLImageElement).src = SANS_POCHETTE)}
            />
          </div>
          <div class="min-w-0 flex-1 flex flex-col gap-0.5">
            <span class="truncate text-[16px] font-extrabold tracking-[-0.01em] text-(--lc-tx)" title={trackTitle}>
              {hasTrack ? trackTitle : $t("player.no_track")}
            </span>
            <span class="truncate text-[13px] font-semibold text-(--lc-acc)">
              {hasTrack ? artistName : "RustMusic"}
            </span>
          </div>
          {@render topActions(false, false)}
        </div>

        <!-- Ligne 2 : Barre de progression + Temps au survol -->
        <div data-no-drag>
          <PlayerProgressBar
            position={percent}
            onseek={(p) => duration && sendCmd("seek", { position: (p / 100) * duration })}
          />
          <div class="flex justify-between font-mono text-[10px] tabular-nums text-(--lc-mu) -mt-0.5 transition-opacity duration-180 {timeVisibleClass}">
            <span>{minutesSeconds(jsPosition)}</span>
            <span>-{minutesSeconds(remaining)}</span>
          </div>
        </div>

        <!-- Ligne 3 : Badge Qualité Audio / Lossless + EqVisualizer centré -->
        <div class="flex items-center justify-center gap-2 min-h-[20px]">
          <EqVisualizer active={isPlaying} />
          {#if hasTrack && (format || qualite)}
            <div class="inline-flex items-center gap-1.5 px-2.5 py-0.5 rounded-full font-mono text-[10px] bg-(--lc-seg) border border-(--lc-seg-bd) text-(--lc-tx2)">
              {#if format}<span class="font-semibold text-(--lc-fmt-tx)">{format}</span>{/if}
              {#if qualite}<span>{qualite}</span>{/if}
              {#if chaine}
                <span class="flex items-center gap-1"><span class="w-1.5 h-1.5 rounded-full {chaine.point}"></span><span>{$t(chaine.cle)}</span></span>
              {/if}
            </div>
          {/if}
        </div>

        <!-- Ligne 4 : Transport 5 boutons -->
        <div class="px-1">
          {@render transportRow(false, false)}
        </div>
      </div>
    {/if}

    <!-- ═════════════════════════════════════════════════════════════════════
         MENU POPUP « … » (Sélecteur Classic | Pill | Sleeve + Tiroirs + Fenêtre)
         ═════════════════════════════════════════════════════════════════════ -->
    {#if menuOpen}
      <div
        class="absolute inset-0 z-40 flex flex-col justify-between p-2.5 bg-zinc-950/92 backdrop-blur-xl text-white
               {$miniPlayerStyle === 'pill' ? 'ml-[58px] rounded-r-[26px] pl-[66px]' : 'rounded-[24px]'}"
        data-no-drag
        transition:fade={{ duration: 130 }}
      >
        <!-- En-tête : Segmented Style Switcher (Classic | Pill | Sleeve) + Fermer menu -->
        <div class="flex items-center gap-1.5">
          <div class="flex-1 grid grid-cols-3 gap-1 p-0.5 rounded-xl bg-white/10 border border-white/12">
            {#each [
              { id: "classic", label: $t("mini.style_classic"), icon: "ph:cards-duotone" },
              { id: "pill", label: $t("mini.style_pill"), icon: "ph:vinyl-record-duotone" },
              { id: "sleeve", label: $t("mini.style_sleeve"), icon: "ph:disc-duotone" },
            ] as st (st.id)}
              {@const activeStyle = $miniPlayerStyle === st.id}
              <button
                type="button"
                class="h-6.5 flex items-center justify-center gap-1 rounded-[9px] text-[11px] font-semibold cursor-pointer transition-all
                       {activeStyle ? 'bg-(--lc-acc) text-zinc-950 shadow-sm' : 'text-white/75 hover:text-white hover:bg-white/8'}"
                onclick={() => selectStyle(st.id as MiniPlayerStyle)}
              >
                <Icon icon={st.icon} width="13" />
                <span>{st.label}</span>
              </button>
            {/each}
          </div>
          <button
            type="button"
            class="w-7 h-7 shrink-0 flex items-center justify-center rounded-full bg-white/12 hover:bg-white/22 text-white cursor-pointer"
            title={$t("common.close")}
            onclick={() => (menuOpen = false)}
          >
            <Icon icon="material-symbols:close-rounded" width="16" />
          </button>
        </div>

        <!-- Sous-variantes Classic (Card | Bar | Large | Micro) -->
        <div class="grid grid-cols-4 gap-1">
          {#each [
            { id: "card", label: $t("mini.layout_card"), icon: "material-symbols-light:crop-portrait-outline" },
            { id: "bar", label: $t("mini.layout_bar"), icon: "material-symbols-light:crop-16-9-outline" },
            { id: "large", label: $t("mini.layout_cover"), icon: "material-symbols-light:image-outline-rounded" },
          ] as lay (lay.id)}
            {@const isCur = $miniPlayerStyle === "classic" && $miniClassicLayout === lay.id}
            <button
              type="button"
              class="h-6.5 flex items-center justify-center gap-1 rounded-lg text-[10.5px] font-medium cursor-pointer border transition-colors
                     {isCur ? 'bg-white/20 border-white/35 text-white font-semibold' : 'bg-white/6 border-white/8 text-white/70 hover:text-white hover:bg-white/12'}"
              onclick={() => selectClassicLayout(lay.id as MiniClassicLayout)}
            >
              <Icon icon={lay.icon} width="13" />
              <span>{lay.label}</span>
            </button>
          {/each}
          <button
            type="button"
            class="h-6.5 flex items-center justify-center gap-1 rounded-lg text-[10.5px] font-medium cursor-pointer border bg-white/6 border-white/8 text-white/70 hover:text-white hover:bg-white/12 transition-colors"
            title={$t("mini.micro")}
            onclick={() => { menuOpen = false; void enterMicroPlayer(); }}
          >
            <Icon icon="ph:picture-in-picture" width="13" />
            <span>{$t("mini.layout_micro")}</span>
          </button>
        </div>

        <!-- Volume rapide (si hors mode Large) -->
        {#if $miniPlayerStyle !== "pill" && !dopActive}
          <div class="flex items-center gap-2 px-2 py-1 rounded-lg bg-white/6 border border-white/8">
            <button type="button" class="text-white/80 hover:text-white cursor-pointer" onclick={toggleMute}>
              <Icon icon={volumeIcon} width="15" />
            </button>
            <input
              type="range"
              min="0"
              max="100"
              step="1"
              value={$volume}
              oninput={(e) => applyVolume(Number((e.currentTarget as HTMLInputElement).value))}
              class="flex-1 h-1 accent-white cursor-pointer"
              aria-label={$t("common.volume")}
            />
            <span class="w-7 text-right font-mono text-[10px] tabular-nums text-white/75">{$volume}%</span>
          </div>
        {/if}

        <!-- Actions rapides : File d'attente, Paroles, Épingler, Plein lecteur, Quitter -->
        <div class="flex items-center justify-between gap-1">
          <div class="flex items-center gap-1">
            <button
              type="button"
              class="h-7 px-2.5 flex items-center gap-1 rounded-lg text-[11px] font-semibold cursor-pointer transition-colors
                     {onglet === 'queue' ? 'bg-(--lc-acc) text-zinc-950' : 'bg-white/10 hover:bg-white/18 text-white'}"
              onclick={() => toggleTab("queue")}
            >
              <Icon icon="material-symbols-light:queue-music-rounded" width="15" />
              <span>{$t("mini.queue_tab")}</span>
              {#if aVenir > 0}
                <span class="px-1 rounded text-[9.5px] font-mono bg-black/25">{aVenir}</span>
              {/if}
            </button>
            <button
              type="button"
              class="h-7 px-2.5 flex items-center gap-1 rounded-lg text-[11px] font-semibold cursor-pointer transition-colors
                     {onglet === 'lyrics' ? 'bg-(--lc-acc) text-zinc-950' : 'bg-white/10 hover:bg-white/18 text-white'}"
              onclick={() => toggleTab("lyrics")}
            >
              <Icon icon="material-symbols-light:lyrics-outline-rounded" width="15" />
              <span>{$t("mini.lyrics")}</span>
            </button>
          </div>

          <div class="flex items-center gap-1">
            <button
              type="button"
              class="w-7 h-7 flex items-center justify-center rounded-lg cursor-pointer transition-colors
                     {$miniPinned ? 'bg-(--lc-acc)/25 text-(--lc-acc) border border-(--lc-acc)/40' : 'bg-white/10 hover:bg-white/18 text-white/80'}"
              title={$t("mini.pin")}
              aria-label={$t("mini.pin")}
              aria-pressed={$miniPinned}
              onclick={toggleMiniPin}
            >
              <Icon icon={$miniPinned ? "material-symbols:keep" : "material-symbols-light:keep-outline"} width="15" />
            </button>
            <button
              type="button"
              class="w-7 h-7 flex items-center justify-center rounded-lg cursor-pointer bg-white/10 hover:bg-white/18 text-white/85"
              title={$t("mini.restore")}
              aria-label={$t("mini.restore")}
              onclick={() => sendCmd("exit-mini")}
            >
              <Icon icon="material-symbols-light:open-in-full-rounded" width="15" />
            </button>
            <button
              type="button"
              class="w-7 h-7 flex items-center justify-center rounded-lg cursor-pointer bg-white/10 hover:bg-[#c42b1c] text-white/85 hover:text-white"
              title={$t("common.close")}
              aria-label={$t("common.close")}
              onclick={fermer}
            >
              <Icon icon="material-symbols-light:close-rounded" width="16" />
            </button>
          </div>
        </div>
      </div>
    {/if}

    <!-- Bulle d'indication du volume lors d'un tour de molette -->
    {#if volumeHint}
      <div
        class="pointer-events-none absolute top-2.5 left-1/2 -translate-x-1/2 z-50 flex items-center gap-1.5 px-3 h-7 rounded-full bg-black/78 backdrop-blur-md border border-white/15 text-white shadow-lg"
        transition:fade={{ duration: 120 }}
      >
        <Icon icon={volumeIcon} width="15" />
        <span class="font-mono text-[11px] tabular-nums">{$volume}%</span>
      </div>
    {/if}
  </div>

  <!-- ═══════════════════════════════════════════════════════════════════════
       TIROIR INFÉRIEUR DÉROULÉ (File d'attente / Paroles)
       ═══════════════════════════════════════════════════════════════════════ -->
  {#if onglet}
    <div
      class="mini-lecteur relative flex-1 min-h-0 flex flex-col border-t border-(--lc-trait) bg-(--lc-panneau) rounded-b-[24px] overflow-hidden"
      data-no-drag
      data-scrollable
      in:fade={{ duration: 160, delay: 40 }}
    >
      <!-- Barre d'en-tête du tiroir (permet de basculer File <-> Paroles ou fermer le tiroir) -->
      <div class="shrink-0 flex items-center justify-between px-3 py-1.5 border-b border-(--lc-trait)">
        <div class="flex items-center gap-1">
          <button
            type="button"
            class="h-6.5 px-2.5 flex items-center gap-1.5 rounded-lg text-[11.5px] font-semibold cursor-pointer transition-colors
                   {onglet === 'queue' ? 'bg-(--lc-seg-on) text-(--lc-tx)' : 'text-(--lc-tx2) hover:text-(--lc-tx)'}"
            onclick={() => (onglet = "queue")}
          >
            <Icon icon="material-symbols-light:queue-music-rounded" width="15" class={onglet === "queue" ? "text-(--lc-acc)" : ""} />
            <span>{$t("mini.queue_tab")}</span>
            {#if aVenir > 0}
              <span class="px-1.5 py-px rounded font-mono text-[9.5px] bg-(--lc-survol) text-(--lc-tx2)">{aVenir}</span>
            {/if}
          </button>
          <button
            type="button"
            class="h-6.5 px-2.5 flex items-center gap-1.5 rounded-lg text-[11.5px] font-semibold cursor-pointer transition-colors
                   {onglet === 'lyrics' ? 'bg-(--lc-seg-on) text-(--lc-tx)' : 'text-(--lc-tx2) hover:text-(--lc-tx)'}"
            onclick={() => (onglet = "lyrics")}
          >
            <Icon icon="material-symbols-light:lyrics-outline-rounded" width="15" class={onglet === "lyrics" ? "text-(--lc-acc)" : ""} />
            <span>{$t("mini.lyrics")}</span>
          </button>
        </div>
        <button
          type="button"
          class="w-6.5 h-6.5 flex items-center justify-center rounded-full cursor-pointer text-(--lc-mu) hover:bg-(--lc-survol) hover:text-(--lc-tx)"
          title={$t("common.close")}
          onclick={() => { onglet = null; setMiniExpanded(false); }}
        >
          <Icon icon="material-symbols:keyboard-arrow-down-rounded" width="18" />
        </button>
      </div>

      {#if onglet === "queue"}
        <div class="flex-1 min-h-0 overflow-y-auto scrollbar-app pb-2" data-scrollable>
          {#if enCours}
            <p class={entetePanneau}>{$t("mini.now_playing")}</p>
            <div class="flex items-center gap-3 mx-1.5 py-1.5 pl-2 pr-2.5 rounded-[10px] bg-(--lc-encours)">
              <span class="w-3.5 shrink-0"></span>
              {@render vignette(enCours.cover)}
              <span class="min-w-0 flex-1">
                <span class="block truncate text-sm font-semibold text-(--lc-acc)">{enCours.title}</span>
                <span class="block truncate text-xs text-(--lc-mu)">{enCours.artist ?? ""}</span>
              </span>
              <EqVisualizer active={isPlaying} />
            </div>
          {/if}

          <div class={entetePanneau}>
            <span>{suite.length === 1 ? $t("mini.next_one") : $t("mini.next_count").replace("{n}", String(suite.length))}</span>
            <div class="flex items-center gap-2.5">
              {#if isShuffled && suite.length > 1}
                <button
                  type="button"
                  class="normal-case tracking-normal text-[11px] font-semibold cursor-pointer text-(--lc-acc) hover:underline"
                  title={$t("queue_panel.reshuffle")}
                  onclick={() => sendCmd("reshuffle")}
                >
                  {$t("queue_panel.reshuffle")}
                </button>
              {/if}
              {#if suite.length > 0 && enCours}
                <button
                  type="button"
                  class="normal-case tracking-normal text-[11px] font-semibold cursor-pointer text-(--lc-mu) hover:text-(--lc-tx)"
                  title={$t("mini.clear_next_hint")}
                  onclick={() => sendCmd("clear-upcoming")}
                >
                  {$t("mini.clear_next")}
                </button>
              {/if}
            </div>
          </div>

          {#if suite.length === 0}
            <p class="px-4 py-4 text-xs text-(--lc-mu)">{$t("mini.queue_empty")}</p>
          {:else}
            {#each suite as track, i (track.queueId)}
              <div
                data-qi={i}
                role="button"
                tabindex="0"
                class="group flex items-center gap-3 mx-1.5 py-1.5 pl-2 pr-2.5 rounded-[10px] cursor-pointer transition-colors hover:bg-(--lc-survol)
                       {depart === i ? 'opacity-50' : ''} {depart !== null && survol === i && depart !== i ? 'shadow-[inset_0_2px_0_var(--lc-acc)]' : ''}"
                onclick={() => playAt(i)}
                onkeydown={(e) => { if (e.key === "Enter") playAt(i); }}
              >
                <span
                  class="w-3.5 shrink-0 flex justify-center text-(--lc-mu2) opacity-0 group-hover:opacity-100 cursor-grab active:cursor-grabbing touch-none"
                  title={$t("mini.move")}
                  role="presentation"
                  onpointerdown={(e) => grab(e, i)}
                  onpointermove={drag}
                  onpointerup={drop}
                  onclick={(e) => e.stopPropagation()}
                >
                  <Icon icon="material-symbols-light:drag-indicator" width="16" />
                </span>
                {@render vignette(track.cover)}
                <span class="min-w-0 flex-1">
                  <span class="block truncate text-sm font-semibold text-(--lc-tx)">{track.title}</span>
                  <span class="block truncate text-xs text-(--lc-mu)">{track.artist ?? ""}</span>
                </span>
                {#if track.duration}
                  <span class="font-mono text-[11px] tabular-nums text-(--lc-mu) group-hover:hidden">{minutesSeconds(track.duration)}</span>
                {/if}
                <button
                  type="button"
                  class="hidden group-hover:flex w-6.5 h-6.5 items-center justify-center rounded-md cursor-pointer text-(--lc-mu) hover:bg-(--lc-survol) hover:text-(--lc-tx)"
                  title={$t("mini.remove")}
                  aria-label={$t("mini.remove")}
                  onclick={(e) => { e.stopPropagation(); sendCmd("remove-track", { queueId: track.queueId }); }}
                >
                  <Icon icon="material-symbols-light:close-rounded" width="16" />
                </button>
              </div>
            {/each}
          {/if}
        </div>
      {:else}
        <div bind:this={boite} class="mini-paroles relative flex-1 min-h-0 overflow-y-auto scrollbar-app" data-scrollable>
          {#if etat === "loading"}
            <p class="py-20 text-center text-xs text-(--lc-mu)">{$t("mini.lyrics_loading")}</p>
          {:else if lignes.length > 0}
            <div class="h-24"></div>
            {#each lignes as ligne, i (i)}
              <button
                type="button"
                bind:this={elLignes[i]}
                class="block w-full text-left px-5 py-1.25 text-[16px] font-bold leading-[1.35] text-pretty origin-left cursor-pointer
                       transition-[color,transform,filter] duration-250 hover:text-(--lc-tx2)
                       {i === active ? 'text-(--lc-tx) scale-104 first-letter:text-(--lc-acc)' : i < active ? 'text-(--lc-ly-passe) blur-[0.4px]' : 'text-(--lc-ly) blur-[0.6px]'}"
                title={$t("mini.jump_line")}
                onclick={() => sendCmd("seek", { position: ligne.timeMs / 1000 })}
              >{ligne.text || "♪"}</button>
            {/each}
            <div class="h-24"></div>
          {:else if texteBrut}
            <p class="px-5 py-4 text-sm leading-relaxed whitespace-pre-wrap text-(--lc-tx2)">{texteBrut}</p>
          {:else}
            <p class="py-20 text-center text-xs text-(--lc-mu)">{$t("mini.lyrics_empty")}</p>
          {/if}
        </div>
        {#if etat === "ready"}
          <span class="shrink-0 h-6 flex items-center px-4 text-[10px] text-(--lc-mu2) border-t border-(--lc-trait)">
            {lignes.length > 0 ? $t("mini.lyrics_synced") : $t("mini.lyrics_plain")}{#if source && SOURCES[source]} · {$t(SOURCES[source])}{/if}
          </span>
        {/if}
      {/if}
    </div>
  {/if}
</div>
