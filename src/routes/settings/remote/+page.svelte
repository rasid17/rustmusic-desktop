<script lang="ts">
  // Mobile Remote (Phone/Tablet via Wi-Fi LAN) & Amplifier Remote Control
  import { onMount, onDestroy } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { t } from "#lib/i18n";
  import OptionGroup from "#lib/components/ui/input/OptionGroup.svelte";
  import OptionItem from "#lib/components/ui/input/OptionItem.svelte";
  import OptionBlock from "#lib/components/ui/input/OptionBlock.svelte";
  import TextField from "#lib/components/ui/input/TextField.svelte";
  import ToggleSwitch from "#lib/components/ui/input/ToggleSwitch.svelte";
  import GhostButton from "#lib/components/ui/button/GhostButton.svelte";
  import Badge from "#lib/components/ui/text/Badge.svelte";
  import { settingsStore } from "#lib/stores/settings/settings.store";
  import { AMP_BRANDS, ampRemoteStore, type Amp } from "#lib/stores/audio/ampRemote.store";
  import type { MobileRemoteStatus } from "#lib/services/remote/mobileRemoteBridge";

  // ─── Mobile Remote state ───
  let mobileStatus = $state<MobileRemoteStatus | null>(null);
  let mobileToggling = $state(false);
  let mobileUpdating = $state(false);
  let mobileCopied = $state(false);
  let refreshTimer: ReturnType<typeof setInterval> | null = null;

  async function refreshMobileStatus() {
    try {
      mobileStatus = await invoke<MobileRemoteStatus>("mobile_remote_status");
    } catch (e) {
      console.error("[mobile-remote] status error:", e);
    }
  }

  onMount(() => {
    void refreshMobileStatus();
    refreshTimer = setInterval(() => {
      if (mobileStatus?.running) void refreshMobileStatus();
    }, 5000);
  });

  onDestroy(() => {
    if (refreshTimer) clearInterval(refreshTimer);
  });

  async function handleMobileToggle() {
    if (mobileToggling) return;
    mobileToggling = true;
    try {
      mobileStatus = mobileStatus?.running
        ? await invoke<MobileRemoteStatus>("mobile_remote_stop")
        : await invoke<MobileRemoteStatus>("mobile_remote_start");
    } catch (e) {
      console.error("[mobile-remote] toggle failed:", e);
    } finally {
      mobileToggling = false;
    }
  }

  async function handleMobilePortSave(raw: string) {
    if (!mobileStatus || mobileUpdating) return;
    const port = parseInt(raw, 10);
    if (!Number.isFinite(port) || port < 1024 || port > 65535 || port === mobileStatus.port) return;
    mobileUpdating = true;
    try {
      mobileStatus = await invoke<MobileRemoteStatus>("mobile_remote_update_settings", {
        port,
        pinEnabled: null,
        pin: null,
      });
    } catch (e) {
      console.error("[mobile-remote] port update failed:", e);
    } finally {
      mobileUpdating = false;
    }
  }

  async function handleMobilePinToggle() {
    if (!mobileStatus || mobileUpdating) return;
    mobileUpdating = true;
    try {
      mobileStatus = await invoke<MobileRemoteStatus>("mobile_remote_update_settings", {
        port: null,
        pinEnabled: !mobileStatus.pin_enabled,
        pin: null,
      });
    } catch (e) {
      console.error("[mobile-remote] pin toggle failed:", e);
    } finally {
      mobileUpdating = false;
    }
  }

  async function handleMobilePinSave(raw: string) {
    if (!mobileStatus || mobileUpdating) return;
    const trimmed = raw.trim();
    if (!trimmed || trimmed === mobileStatus.pin) return;
    mobileUpdating = true;
    try {
      mobileStatus = await invoke<MobileRemoteStatus>("mobile_remote_update_settings", {
        port: null,
        pinEnabled: null,
        pin: trimmed,
      });
    } catch (e) {
      console.error("[mobile-remote] pin save failed:", e);
    } finally {
      mobileUpdating = false;
    }
  }

  async function handleCopyMobileUrl() {
    if (!mobileStatus?.url) return;
    const fullUrl = mobileStatus.pin_enabled
      ? `${mobileStatus.url}/?pin=${encodeURIComponent(mobileStatus.pin)}`
      : mobileStatus.url;
    try {
      await navigator.clipboard.writeText(fullUrl);
      mobileCopied = true;
      setTimeout(() => (mobileCopied = false), 1500);
    } catch (e) {
      console.error("[mobile-remote] clipboard failed:", e);
    }
  }

  // ─── Amplifier Remote state ───
  const actif = $derived($settingsStore.amp_remote === "true");
  const pilote = $derived($ampRemoteStore.view);

  $effect(() => {
    if (actif && !$ampRemoteStore.searched && !$ampRemoteStore.searching) ampRemoteStore.discover();
  });

  const amplis = $derived.by(() => {
    const liste = [...$ampRemoteStore.found];
    if (pilote && !liste.some((a) => a.id === pilote.amp.id)) liste.unshift(pilote.amp);
    return liste;
  });

  async function selectAmp(amp: Amp | null) {
    try {
      await ampRemoteStore.select(amp);
    } catch (e) {
      console.error("[télécommande] choix :", e);
    }
  }
</script>

<!-- ─── Mobile Remote (Phone & Tablet via Wi-Fi / LAN) ─── -->
<OptionGroup
  title="Mobile Remote (Phone & Tablet)"
  hint="Control playback, browse tracks, albums, artists, queue, and playlists from your phone over Wi-Fi without affecting Bit-Perfect audio output."
>
  <OptionItem
    title="Enable Mobile Web Remote"
    desc="Starts a lightweight local web server on your Wi-Fi / LAN. Scan the QR code with your phone camera to connect."
    keywords="mobile phone remote wifi lan qr pwa android ios browse queue playlist"
    onclick={mobileToggling ? undefined : handleMobileToggle}
  >
    <ToggleSwitch
      checked={mobileStatus?.running ?? false}
      disabled={mobileToggling}
      label="Enable Mobile Web Remote"
      onclick={handleMobileToggle}
    />
  </OptionItem>

  {#if mobileStatus?.running && mobileStatus.url}
    <OptionBlock keywords="qr code mobile url lan address" class="flex flex-col sm:flex-row items-center gap-5 px-5 py-5">
      {#if mobileStatus.qr_svg}
        <div class="w-36 h-36 shrink-0 rounded-2xl p-2 bg-white shadow-md border border-(--rg-bd)">
          {@html mobileStatus.qr_svg}
        </div>
      {/if}
      <div class="flex-1 min-w-0 space-y-2 text-center sm:text-left">
        <div class="flex items-center justify-center sm:justify-start gap-2">
          <span class="relative shrink-0 flex w-2.5 h-2.5">
            <span class="absolute inset-0 rounded-full bg-(--rg-g) opacity-50 animate-ping"></span>
            <span class="relative w-2.5 h-2.5 rounded-full bg-(--rg-g)"></span>
          </span>
          <p class="text-[15px] font-bold text-(--rg-gtx)">Mobile Remote Active</p>
          <span class="text-xs px-2 py-0.5 rounded-full bg-(--rg-s2) text-(--rg-tx2) font-semibold">
            {mobileStatus.connected_clients} {mobileStatus.connected_clients === 1 ? "device" : "devices"}
          </span>
        </div>
        <p class="font-mono text-sm text-(--rg-tx) select-all break-all">
          {mobileStatus.url}{mobileStatus.pin_enabled ? `/?pin=${mobileStatus.pin}` : ""}
        </p>
        <p class="text-xs text-(--rg-mu)">
          Scan the QR code with your phone camera (on the same Wi-Fi / Hotspot) or open the URL in Safari / Chrome. You can also choose "Add to Home Screen" for a full-screen app experience.
        </p>
        <div class="pt-1 flex items-center justify-center sm:justify-start gap-2">
          <GhostButton
            icon={mobileCopied ? "material-symbols:check-rounded" : "material-symbols:content-copy-outline-rounded"}
            label={mobileCopied ? "Copied!" : "Copy Remote URL"}
            onclick={handleCopyMobileUrl}
          />
        </div>
      </div>
    </OptionBlock>
  {/if}

  {#if mobileStatus}
    <OptionItem
      title="Require PIN Code"
      desc="Protect remote access on shared Wi-Fi networks with a PIN code (automatically embedded in the QR code)."
      keywords="pin password security remote"
      onclick={mobileUpdating ? undefined : handleMobilePinToggle}
    >
      <ToggleSwitch
        checked={mobileStatus.pin_enabled}
        disabled={mobileUpdating}
        label="Require PIN Code"
        onclick={handleMobilePinToggle}
      />
    </OptionItem>

    {#if mobileStatus.pin_enabled}
      <OptionItem title="Remote PIN" desc="4 to 8 characters required to unlock the remote on a new device.">
        <TextField
          value={mobileStatus.pin}
          label="Remote PIN"
          maxlength={8}
          mono
          class="w-28"
          onchange={handleMobilePinSave}
        />
      </OptionItem>
    {/if}

    <OptionItem title="HTTP Port" desc="Local TCP port used by the Mobile Remote web server (default: 8300)." keywords="port http">
      <TextField
        type="number"
        value={mobileStatus.port}
        label="HTTP Port"
        min={1024}
        max={65535}
        mono
        class="w-24"
        onchange={handleMobilePortSave}
      />
    </OptionItem>
  {/if}
</OptionGroup>

<!-- ─── Amplifier Remote ─── -->
<OptionGroup title={$t("amp_remote.group")} hint={$t("amp_remote.group_hint")}>
  {#snippet action()}<Badge tone="amber">{$t("settings.beta_badge")}</Badge>{/snippet}
  <OptionItem
    title={$t("amp_remote.toggle")}
    desc={$t("amp_remote.toggle_desc")}
    keywords="onkyo pioneer integra denon marantz yamaha ampli télécommande remote"
    onclick={() => settingsStore.toggle("amp_remote")}
  >
    <ToggleSwitch checked={actif} label={$t("amp_remote.toggle")} onclick={() => settingsStore.toggle("amp_remote")} />
  </OptionItem>
</OptionGroup>

{#if actif}
  <OptionGroup title={$t("amp_remote.amps")} hint={$t("amp_remote.amps_hint")}>
    {#snippet action()}
      <GhostButton
        icon="material-symbols:refresh-rounded"
        label={$ampRemoteStore.searching ? $t("network_player.searching") : $t("network_player.search")}
        busy={$ampRemoteStore.searching}
        disabled={$ampRemoteStore.searching}
        onclick={() => ampRemoteStore.discover()}
      />
    {/snippet}

    {#if pilote}
      <OptionBlock keywords={pilote.amp.name} class="flex items-center gap-4 px-5 py-4 leading-[1.2]">
        <span class="relative shrink-0 flex w-2.5 h-2.5">
          {#if pilote.state.connected}<span class="absolute inset-0 rounded-full bg-(--rg-g) opacity-50 animate-ping"></span>{/if}
          <span class="relative w-2.5 h-2.5 rounded-full {pilote.state.connected ? 'bg-(--rg-g)' : 'bg-(--rg-am)'}"></span>
        </span>
        <div class="flex-1 min-w-0">
          <p class="text-[15px] font-semibold {pilote.state.connected ? 'text-(--rg-gtx)' : 'text-(--rg-am)'}">
            {(pilote.state.connected ? $t("amp_remote.connected") : $t("amp_remote.unreachable")).replace("{name}", pilote.amp.name)}
          </p>
          <p class="mt-0.75 text-[13px] text-(--rg-mu) truncate">{AMP_BRANDS[pilote.amp.brand]} · {pilote.amp.ip}</p>
        </div>
        <GhostButton icon="material-symbols:link-off-rounded" label={$t("amp_remote.stop")} onclick={() => selectAmp(null)} />
      </OptionBlock>
    {/if}

    {#each amplis.filter((a) => a.id !== pilote?.amp.id) as amp (amp.id)}
      <OptionItem title={amp.name} desc="{AMP_BRANDS[amp.brand]} · {amp.model} · {amp.ip}">
        <GhostButton icon="material-symbols:settings-remote-outline-rounded" label={$t("amp_remote.control")} onclick={() => selectAmp(amp)} />
      </OptionItem>
    {:else}
      {#if !pilote && $ampRemoteStore.searched && !$ampRemoteStore.searching}
        <OptionBlock keywords={$t("amp_remote.none")} class="px-5 py-4 text-[13px] text-(--rg-mu)">{$t("amp_remote.none")}</OptionBlock>
      {/if}
    {/each}
  </OptionGroup>
{/if}
