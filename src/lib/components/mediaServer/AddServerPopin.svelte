<script lang="ts">
  // Ajout d'un serveur (OpenSubsonic, Jellyfin ou DLNA) : connexion, puis choix des bibliothèques à ajouter.
  import { onDestroy } from "svelte";
  import Icon from "@iconify/svelte";
  import { openUrl } from "@tauri-apps/plugin-opener";
  import { t } from "#lib/i18n";
  import { popinStore } from "#lib/stores/ui/popin.store";
  import { serverStore, type ServerCredentials } from "#lib/stores/mediaServer/mediaServer.store";
  import { toasts } from "#lib/stores/ui/toast.store";
  import { errorMessage } from "#lib/helper/tools/errorTools";
  import FormField from "#lib/components/ui/input/FormField.svelte";
  import FormInput from "#lib/components/ui/input/FormInput.svelte";
  import PopinFooter from "#lib/components/ui/popin/PopinFooter.svelte";
  import ChoiceTile from "#lib/components/ui/input/ChoiceTile.svelte";
  import GhostButton from "#lib/components/ui/button/GhostButton.svelte";
  import DlnaFolderPicker from "#lib/components/mediaServer/DlnaFolderPicker.svelte";
  import type { DlnaFound, ServerKind, ServerProbe, TidalDeviceAuth } from "#lib/types/ui/mediaServer/MediaServer";

  let kind = $state<ServerKind>("subsonic");
  const jellyfin = $derived(kind === "jellyfin");
  const dlna = $derived(kind === "dlna");
  const qobuz = $derived(kind === "qobuz");
  const tidal = $derived(kind === "tidal");
  let url = $state("");
  let username = $state("");
  let password = $state("");
  let apiKey = $state("");
  let avecCle = $state(false);
  let tidalAuth = $state<TidalDeviceAuth | null>(null);
  let tidalStarting = $state(false);
  let pollTimer: ReturnType<typeof setInterval> | null = null;

  function stopTidalPoll() {
    if (pollTimer) {
      clearInterval(pollTimer);
      pollTimer = null;
    }
  }
  onDestroy(stopTidalPoll);

  let probe = $state<ServerProbe | null>(null);
  let name = $state("");
  let choisis = $state<string[]>([]);
  /** DLNA : dossiers cochés, pris n'importe où dans l'arborescence. */
  let dossiers = $state<{ id: string; name: string }[]>([]);
  let busy = $state(false);

  // DLNA : les serveurs du réseau, cherchés dès que la tuile est choisie.
  let trouves = $state<DlnaFound[]>([]);
  let recherche = $state(false);
  async function discover() {
    recherche = true;
    try {
      trouves = await serverStore.discover();
    } catch {
      trouves = [];
    } finally {
      recherche = false;
    }
  }
  function choose(k: ServerKind) {
    if (busy) return;
    stopTidalPoll();
    tidalAuth = null;
    kind = k;
    erreur = null;
    if (k === "dlna" && !recherche && trouves.length === 0) void discover();
  }
  let erreur = $state<string | null>(null);

  const credentials = (): ServerCredentials => ({
    kind,
    url: qobuz ? "https://www.qobuz.com/api.json/0.2" : tidal ? "https://api.tidal.com/v1" : url.trim(),
    username: username.trim(),
    password: tidal ? (avecCle ? password.trim() : password) : avecCle ? null : password,
    apiKey: avecCle && apiKey.trim() ? apiKey.trim() : null,
  });

  async function startTidalLogin() {
    stopTidalPoll();
    erreur = null;
    tidalStarting = true;
    try {
      const customClient = avecCle && apiKey.trim() ? apiKey.trim() : null;
      const auth = await serverStore.tidalDeviceStart(customClient);
      tidalAuth = auth;
      void openUrl(auth.verificationUriComplete).catch(() => {});
      const intervalMs = Math.max(2000, (auth.interval || 2) * 1000);
      pollTimer = setInterval(async () => {
        if (!tidalAuth || tidalAuth.deviceCode !== auth.deviceCode) return;
        try {
          const poll = await serverStore.tidalDevicePoll(auth.deviceCode, customClient);
          if (poll.status === "authorized" && poll.accessToken) {
            stopTidalPoll();
            tidalAuth = null;
            username = poll.username || poll.userId || "tidal";
            password = `tokens:${poll.accessToken}|${poll.refreshToken ?? ""}`;
            avecCle = false;
            await connect();
          }
        } catch (err) {
          stopTidalPoll();
          tidalAuth = null;
          erreur = errorMessage(err);
        }
      }, intervalMs);
    } catch (err) {
      erreur = errorMessage(err);
    } finally {
      tidalStarting = false;
    }
  }

  async function connect() {
    erreur = null;
    const missing = dlna
      ? false
      : qobuz
        ? avecCle
          ? !apiKey.trim()
          : !username.trim() || !password
        : tidal
          ? !password.trim()
          : avecCle
            ? !apiKey.trim() || (jellyfin && !username.trim())
            : !username.trim() || (!jellyfin && !password);
    if ((!qobuz && !tidal && !url.trim()) || missing) {
      erreur = $t("servers.fill_fields");
      return;
    }
    busy = true;
    try {
      probe = await serverStore.probe(credentials());
      name = probe.name;
      choisis = dlna ? [] : probe.folders.map((f) => f.id);
      dossiers = [];
    } catch (err) {
      erreur = errorMessage(err);
    } finally {
      busy = false;
    }
  }

  async function add() {
    if (dlna) choisis = dossiers.map((f) => f.id);
    if (!probe || choisis.length === 0) return;
    erreur = null;
    busy = true;
    try {
      const s = await serverStore.add({ ...credentials(), url: probe.url }, name.trim() || null, choisis);
      toasts.push({ type: "success", title: $t("servers.added").replace("{name}", s.name) });
      popinStore.close();
    } catch (err) {
      erreur = errorMessage(err);
    } finally {
      busy = false;
    }
  }

  function toggle(id: string) {
    choisis = choisis.includes(id) ? choisis.filter((x) => x !== id) : [...choisis, id];
  }

  const typeServeur = $derived(
    probe
      ? [
          probe.info.kind ? probe.info.kind.charAt(0).toUpperCase() + probe.info.kind.slice(1) : null,
          probe.info.version?.split(" ")[0],
          kind !== "subsonic" ? null : probe.info.open_subsonic ? "OpenSubsonic" : "Subsonic",
        ].filter(Boolean).join(" · ")
      : "",
  );
</script>

<div class="space-y-5">
  {#if !probe}
    <div class="grid grid-cols-2 sm:grid-cols-4 gap-2.5" role="radiogroup" aria-label={$t("servers.kind")}>
      {#each [
        ["subsonic", "servers.kind_subsonic", "servers.intro", true],
        ["jellyfin", "Jellyfin", "servers.intro_jellyfin", true],
        ["dlna", "DLNA", "servers.intro_dlna", true],
        ["qobuz", "Qobuz", "Hi-Res 24-bit / 192 kHz · Bit-Perfect FLAC", false],
        ["tidal", "TIDAL", "Hi-Res MAX 24-bit / 192 kHz · Bit-Perfect", false],
      ] as [value, title, desc, translated] (value)}
        <ChoiceTile
          title={value === "subsonic" ? $t(title as string) : (title as string)}
          desc={translated ? $t(desc as string) : (desc as string)}
          selected={kind === value}
          onclick={() => choose(value as ServerKind)}
        />
      {/each}
    </div>

    {#if dlna}
      <!-- Serveurs annoncés sur le réseau -->
      <div class="flex flex-col gap-2">
        <div class="flex items-center gap-2">
          <p class="flex-1 text-[11px] font-bold uppercase tracking-[0.1em] text-(--rg-mu)">{$t("servers.dlna_found")}</p>
          <GhostButton icon="material-symbols:refresh-rounded" label={$t("servers.dlna_search_again")} busy={recherche} disabled={busy || recherche} onclick={discover} />
        </div>
        {#if recherche && trouves.length === 0}
          <p class="flex items-center gap-2 text-sm text-(--rg-mu)">
            <Icon icon="material-symbols:progress-activity" width="16" class="animate-spin" />{$t("servers.dlna_searching")}
          </p>
        {:else}
          {#each trouves as s (s.id)}
            <button type="button" disabled={busy} onclick={() => { url = s.location; void connect(); }}
                    class="flex items-center gap-3 px-3.5 py-2.5 rounded-xl border text-left cursor-pointer transition-colors border-(--rg-bd) hover:border-(--rg-gbd) hover:bg-(--rg-gbg)">
              <Icon icon="material-symbols:dns-outline-rounded" width="20" class="shrink-0 text-(--rg-mu)" />
              <span class="flex-1 min-w-0">
                <span class="block truncate text-sm font-semibold text-(--rg-tx)">{s.name}</span>
                <span class="block truncate text-xs text-(--rg-mu)">{[s.model, new URL(s.location).host].filter(Boolean).join(" · ")}</span>
              </span>
              <Icon icon="material-symbols:chevron-right-rounded" width="20" class="shrink-0 text-(--rg-mu)" />
            </button>
          {:else}
            <p class="text-sm text-(--rg-mu)">{$t("servers.dlna_none")}</p>
          {/each}
        {/if}
      </div>
    {/if}

    {#if qobuz}
      <div class="flex items-start gap-3 px-4 py-3 rounded-xl border border-amber-500/30 bg-amber-500/8 text-xs leading-relaxed text-(--rg-tx2)">
        <Icon icon="material-symbols:high-quality-outline-rounded" width="20" class="shrink-0 mt-0.5 text-amber-500" />
        <div>
          <p class="font-semibold text-(--rg-tx)">Qobuz Bit-Perfect Streaming (up to 24-bit / 192 kHz)</p>
          <p class="mt-0.5 text-(--rg-mu)">
            Uncompressed FLAC streams are decoded directly by Symphonia and sent to your exclusive audio output (ALSA / WASAPI / CoreAudio) without resampling.
          </p>
        </div>
      </div>
    {:else if tidal}
      <div class="px-4 py-3 rounded-xl border border-cyan-500/30 bg-cyan-500/8 text-xs leading-relaxed text-(--rg-tx2) flex gap-2.5">
        <Icon icon="material-symbols:high-quality-outline-rounded" width="18" class="shrink-0 mt-0.5 text-cyan-500" />
        <div>
          <p class="font-semibold text-(--rg-tx)">TIDAL Bit-Perfect MAX Streaming (up to 24-bit / 192 kHz)</p>
          <p class="mt-0.5 text-(--rg-mu)">
            Uncompressed FLAC and Hi-Res DASH FLAC streams are decoded directly by Symphonia and sent to your exclusive audio output without resampling.
          </p>
        </div>
      </div>
    {:else}
      <FormField label={$t("servers.address")} for="srv_url" required={!dlna}>
        <FormInput id="srv_url" bind:value={url} mono placeholder={dlna ? "http://nas.local:8200" : jellyfin ? "http://nas.local:8096" : "https://music.example.com"} disabled={busy} oninput={() => (erreur = null)} onenter={connect} />
      </FormField>
    {/if}

    {#if dlna}
      <p class="flex gap-2 text-xs leading-relaxed text-(--rg-mu)">
        <Icon icon="material-symbols:info-outline-rounded" width="15" class="shrink-0 mt-0.5" />
        {$t("servers.dlna_address_note")}
      </p>
    {:else if tidal && !avecCle}
      <div class="rounded-xl border border-(--rg-bd) bg-(--rg-s1) p-4 flex flex-col gap-3">
        {#if !tidalAuth}
          <div class="flex flex-wrap items-center justify-between gap-3">
            <div>
              <p class="text-sm font-semibold text-(--rg-tx)">Authorize via TIDAL Browser Login</p>
              <p class="text-xs text-(--rg-mu) mt-0.5">Opens your browser to approve this device using OAuth 2.0 Device Flow.</p>
            </div>
            <GhostButton
              icon={tidalStarting ? "material-symbols:sync-rounded" : "material-symbols:login-rounded"}
              label={tidalStarting ? "Starting..." : "Sign in with TIDAL"}
              busy={tidalStarting}
              disabled={busy || tidalStarting}
              onclick={startTidalLogin}
            />
          </div>
        {:else}
          <div class="flex flex-col gap-2.5">
            <div class="flex flex-wrap items-center justify-between gap-2">
              <div class="flex items-center gap-2 text-xs text-(--rg-tx2)">
                <Icon icon="material-symbols:sync-rounded" width="16" class="animate-spin text-cyan-500" />
                <span>Waiting for browser authorization...</span>
              </div>
              <button
                type="button"
                class="text-xs font-semibold text-(--rg-mu) hover:text-(--rg-tx) cursor-pointer"
                onclick={() => { stopTidalPoll(); tidalAuth = null; }}
              >
                Cancel
              </button>
            </div>
            <div class="flex flex-wrap items-center justify-between gap-3 px-3.5 py-2.5 rounded-lg bg-(--rg-s2)">
              <div>
                <p class="text-[11px] uppercase tracking-wider text-(--rg-mu)">Device Code</p>
                <p class="text-lg font-mono font-bold tracking-widest text-(--rg-tx)">{tidalAuth.userCode}</p>
              </div>
              <GhostButton
                icon="material-symbols:open-in-new-rounded"
                label="Open TIDAL Login"
                onclick={() => openUrl(tidalAuth!.verificationUriComplete)}
              />
            </div>
          </div>
        {/if}
      </div>
    {:else if tidal && avecCle}
      <div class="grid gap-4 max-sm:grid-cols-1 grid-cols-2">
        <FormField label="Access Token or Refresh Token" for="srv_tidal_tok" required>
          <FormInput id="srv_tidal_tok" bind:value={password} mono password placeholder="Paste TIDAL OAuth token" disabled={busy} oninput={() => (erreur = null)} onenter={connect} />
        </FormField>
        <FormField label="Custom Client ID:Secret (optional)" for="srv_key">
          <FormInput id="srv_key" bind:value={apiKey} mono password placeholder="Leave blank for default" disabled={busy} oninput={() => (erreur = null)} onenter={connect} />
        </FormField>
      </div>
    {:else if avecCle}
      <div class="grid gap-4 max-sm:grid-cols-1 {jellyfin || qobuz ? 'grid-cols-2' : ''}">
        {#if jellyfin || qobuz}
          <FormField label={qobuz ? "Email or account label (optional)" : $t("servers.username")} for="srv_user" required={jellyfin}>
            <FormInput id="srv_user" bind:value={username} mono placeholder={qobuz ? "audiophile@example.com" : ""} disabled={busy} oninput={() => (erreur = null)} onenter={connect} />
          </FormField>
        {/if}
        <FormField label={qobuz ? "Qobuz Token (user_auth_token)" : $t("servers.api_key")} for="srv_key" required>
          <FormInput id="srv_key" bind:value={apiKey} mono password disabled={busy} oninput={() => (erreur = null)} onenter={connect} />
        </FormField>
      </div>
    {:else}
      <div class="grid grid-cols-2 gap-4 max-sm:grid-cols-1">
        <FormField label={qobuz ? "Qobuz Email or Username" : $t("servers.username")} for="srv_user" required>
          <FormInput id="srv_user" bind:value={username} mono placeholder={qobuz ? "audiophile@example.com" : ""} disabled={busy} oninput={() => (erreur = null)} onenter={connect} />
        </FormField>
        <FormField label={$t("servers.password")} for="srv_pw" required={!jellyfin}>
          <FormInput id="srv_pw" bind:value={password} password disabled={busy} oninput={() => (erreur = null)} onenter={connect} />
        </FormField>
      </div>
    {/if}

    {#if !dlna}
    <div class="flex flex-wrap items-start gap-x-4 gap-y-2">
      <p class="flex-1 min-w-60 flex gap-2 text-xs leading-relaxed text-(--rg-mu)">
        <Icon icon="material-symbols:lock-outline" width="15" class="shrink-0 mt-0.5" />
        {#if qobuz}
          {avecCle
            ? "Tip: You can paste your user_auth_token (or app_id:app_secret:user_auth_token) from an active play.qobuz.com session."
            : "Your password is only used to request a Qobuz session token; it is never stored on disk."}
        {:else if tidal}
          {avecCle
            ? "Paste an existing TIDAL OAuth access_token or refresh_token. Refresh tokens renew automatically when expired."
            : "OAuth Device Flow stores only session tokens and never asks for your TIDAL password."}
        {:else}
          {$t(`servers.${avecCle ? "api_key_note" : "password_note"}${jellyfin ? "_jellyfin" : ""}`)}
        {/if}
      </p>
      <button type="button" onclick={() => { stopTidalPoll(); tidalAuth = null; avecCle = !avecCle; erreur = null; }}
              class="text-xs font-semibold cursor-pointer text-(--rg-gtx) hover:underline underline-offset-2">
        {qobuz
          ? avecCle
            ? "Use email & password"
            : "Use token (user_auth_token)"
          : tidal
            ? avecCle
              ? "Use Browser OAuth Login"
              : "Use token (access / refresh)"
            : avecCle
              ? $t("servers.use_password")
            : $t("servers.use_api_key")}
      </button>
    </div>
    {/if}
  {:else}
    <!-- Le serveur tel qu'il apparaîtra -->
    <div class="flex items-center gap-4 p-3.5 rounded-2xl border border-sky-500/30 bg-linear-135 from-sky-500/10 to-transparent">
      <span class="w-12 h-12 shrink-0 rounded-xl flex items-center justify-center bg-sky-500/15 border border-sky-500/30 text-sky-600 dark:text-sky-300">
        <Icon icon="material-symbols:cloud-outline-rounded" width="26" />
      </span>
      <div class="min-w-0">
        <p class="text-[15px] font-bold truncate text-(--rg-tx)">{name.trim() || probe.name}</p>
        <p class="mt-0.5 text-xs truncate text-(--rg-mu)">{typeServeur} · {probe.url}</p>
      </div>
      <Icon icon="material-symbols:check-circle-rounded" width="22" class="ml-auto shrink-0 text-(--rg-g)" />
    </div>

    <FormField label={$t("servers.name")} for="srv_name">
      <FormInput id="srv_name" bind:value={name} placeholder={probe.name} disabled={busy} onenter={add} />
    </FormField>

    <div class="flex flex-col gap-2">
      <p class="text-[11px] font-bold uppercase tracking-[0.1em] text-(--rg-mu)">{$t(dlna ? "servers.dlna_choose" : "servers.choose_libraries")}</p>
      {#if dlna}
        <DlnaFolderPicker url={probe.url} bind:selected={dossiers} disabled={busy} />
      {:else if probe.folders.length === 0}
        <p class="text-sm text-(--rg-mu)">{$t("servers.no_folders")}</p>
      {:else}
        <div class="flex flex-col gap-1.5">
          {#each probe.folders as f (f.id)}
            <label class="flex items-center gap-3 px-3.5 py-2.5 rounded-xl border cursor-pointer transition-colors
                          {choisis.includes(f.id) ? 'border-(--rg-gbd) bg-(--rg-gbg)' : 'border-(--rg-bd) hover:border-(--rg-bd2)'}">
              <input type="checkbox" class="checkbox-app" checked={choisis.includes(f.id)} onchange={() => toggle(f.id)} disabled={busy} />
              <Icon icon="material-symbols:cloud-outline-rounded" width="18" class="shrink-0 text-(--rg-mu)" />
              <span class="flex-1 min-w-0 truncate text-sm font-semibold text-(--rg-tx)">{f.name}</span>
            </label>
          {/each}
        </div>
      {/if}
      <p class="text-xs leading-relaxed text-(--rg-mu)">{$t(dlna ? "servers.dlna_choose_hint" : "servers.libraries_note")}</p>
    </div>
  {/if}

  {#if erreur}
    <p class="flex items-center gap-2 px-4 py-3 rounded-xl border border-red-500/25 bg-red-500/8 text-sm text-red-600 dark:text-red-400">
      <Icon icon="material-symbols:warning-outline-rounded" width="17" class="shrink-0" />
      {erreur}
    </p>
  {/if}
</div>

<PopinFooter
  bleed
  cancelLabel={probe ? $t("servers.back") : $t("common.cancel")}
  oncancel={() => (probe ? ((probe = null), (erreur = null)) : popinStore.close())}
  cancelDisabled={busy}
  submitLabel={probe ? $t("servers.add_libraries").replace("{n}", String(dlna ? dossiers.length : choisis.length)) : $t("servers.connect")}
  submitIcon={probe ? "material-symbols:add-rounded" : "material-symbols:link-rounded"}
  onsubmit={probe ? add : connect}
  submitDisabled={(!probe && tidal && !avecCle && !password.trim()) || (!!probe && (dlna ? dossiers.length : choisis.length) === 0)}
  busy={busy}
/>
