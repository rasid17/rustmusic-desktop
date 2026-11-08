<script lang="ts">
  import { readableSize } from "#lib/helper/tools/sizeTools";
  import Icon from "@iconify/svelte";
  import { forgetLocations } from "#lib/helper/library/trackLocation";
  import { invoke } from "@tauri-apps/api/core";
  import { handleAddDirectory } from "#lib/actions/library/LibraryAction";
  import { libraryContentStore } from "#lib/stores/library/libraryContent.store";
  import { libraryStore } from "#lib/stores/library/library.store";
  import { importProgressStore, importsTermines } from "#lib/stores/library/importProgress.store";
  import ToggleSwitch from "#lib/components/ui/input/ToggleSwitch.svelte";
  import { toasts } from "#lib/stores/ui/toast.store";
  import { portal } from "#lib/helper/portal";
  import { t, currentLocale } from "#lib/i18n";
  import { shortSince } from "#lib/helper/tools/dateTools";
  import {
    getFolderProfiles,
    saveFolderProfiles,
    applyFolderProfile,
    type FolderProfile,
  } from "#lib/helper/library/folderProfiles";

  type LibraryDir = {
    id: string;
    library_id: number;
    path: string;
    name: string;
    is_recursive: boolean;
    is_active: boolean;
    watch_enabled: boolean;
    total_files: number;
    total_size: number;
    last_scan_at: string | null;
    scan_status: string;
    created_at: string;
  };

  let { open = $bindable(true), libraryId }: { open: boolean; libraryId: number } = $props();

  let dirs: LibraryDir[] = $state([]);
  let loading = $state(true);
  let rescanningId: string | null = $state(null);
  let profiles: FolderProfile[] = $state([]);
  let isCreatingPreset = $state(false);
  let newPresetName = $state('');
  let applyingBatch = $state(false);

  $effect(() => {
    if (open) {
      loadDirs();
      profiles = getFolderProfiles(libraryId);
    }
  });

  const imp = $derived($importProgressStore);
  const importIci = $derived(imp.active && imp.libraryId === libraryId);

  // Recharge quand un dossier inconnu commence à s'importer, puis à la fin de chaque dossier.
  let dossierSuivi = '';
  $effect(() => {
    const dossier = importIci ? imp.directory : '';
    if (dossier === dossierSuivi) return;
    const fini = dossierSuivi !== '';
    dossierSuivi = dossier;
    if (open && (fini || !dirs.some(d => d.path === dossier))) loadDirs(false);
  });

  async function loadDirs(attente = true) {
    if (attente) loading = true;
    try {
      dirs = await invoke('get_library_dirs', { libraryId });
    } catch (e) {
      console.error('Failed to load dirs:', e);
    } finally {
      loading = false;
    }
  }

  function reportBusy(e: unknown): boolean {
    if (e !== 'deja_en_cours') return false;
    toasts.push({ type: "error", title: $t("notify.error"), message: $t("notify.import_busy") });
    return true;
  }

  async function handleRescanDir(dir: LibraryDir) {
    rescanningId = dir.id;
    try {
      await invoke('rescan_library_dir', { libraryId, dirId: dir.id });
      forgetLocations();
      await loadDirs();
      libraryContentStore.load(libraryId);
    } catch (e) {
      if (!reportBusy(e)) console.error('Rescan failed:', e);
    } finally {
      rescanningId = null;
    }
  }

  async function handleToggleDirActive(dir: LibraryDir) {
    const nextActive = !dir.is_active;
    try {
      await invoke('set_library_dir_active', { dirId: dir.id, isActive: nextActive });
      dir.is_active = nextActive;
      forgetLocations();
      await Promise.all([
        libraryContentStore.refresh(libraryId),
        libraryStore.refresh(),
      ]);
      importsTermines.update(n => n + 1);
    } catch (e) {
      console.error('Toggle active failed:', e);
    }
  }

  async function handleToggleDirWatch(dir: LibraryDir) {
    const nextWatch = !dir.watch_enabled;
    try {
      await invoke('set_library_dir_watch', { dirId: dir.id, watchEnabled: nextWatch });
      dir.watch_enabled = nextWatch;
    } catch (e) {
      console.error('Toggle watch failed:', e);
    }
  }

  function handleSavePreset() {
    const name = newPresetName.trim();
    if (!name) return;
    const activeDirIds = dirs.filter(d => d.is_active).map(d => d.id);
    const newProfile: FolderProfile = {
      id: typeof crypto !== 'undefined' && crypto.randomUUID ? crypto.randomUUID() : String(Date.now()),
      name,
      activeDirIds,
    };
    profiles = [...profiles, newProfile];
    saveFolderProfiles(libraryId, profiles);
    newPresetName = '';
    isCreatingPreset = false;
  }

  function handleDeletePreset(e: MouseEvent, profileId: string) {
    e.stopPropagation();
    profiles = profiles.filter(p => p.id !== profileId);
    saveFolderProfiles(libraryId, profiles);
  }

  async function handleApplyBatch(activeDirIds: string[]) {
    if (applyingBatch) return;
    applyingBatch = true;
    try {
      await applyFolderProfile(libraryId, activeDirIds);
      for (const d of dirs) {
        d.is_active = activeDirIds.includes(d.id);
      }
    } catch (e) {
      console.error('Apply batch failed:', e);
    } finally {
      applyingBatch = false;
    }
  }

  const allActive = $derived(dirs.length > 0 && dirs.every(d => d.is_active));

  function isProfileActive(profile: FolderProfile): boolean {
    const activeDirs = dirs.filter(d => d.is_active);
    return (
      activeDirs.length === profile.activeDirIds.length &&
      activeDirs.every(d => profile.activeDirIds.includes(d.id))
    );
  }

  async function handleRemoveDir(dir: LibraryDir) {
    try {
      await invoke('remove_library_dir', { dirId: dir.id });
      dirs = dirs.filter(d => d.id !== dir.id);
      forgetLocations();
      await Promise.all([
        libraryContentStore.refresh(libraryId),
        libraryStore.refresh(),
      ]);
      importsTermines.update(n => n + 1);
    } catch (e) {
      console.error('Remove failed:', e);
    }
  }

  async function handleAddFolder() {
    await handleAddDirectory(libraryId);
    await loadDirs();
  }

  const formatSize = (bytes: number) => readableSize(bytes, $currentLocale);

  function nb(n: number, un: string, plusieurs: string): string {
    return n === 1 ? $t(un) : $t(plusieurs).replace('{n}', n.toLocaleString($currentLocale));
  }

  const totalFichiers = $derived(dirs.reduce((s, d) => s + (d.total_files ?? 0), 0));
  const totalTaille = $derived(dirs.reduce((s, d) => s + (d.total_size ?? 0), 0));
  const resume = $derived([
    nb(dirs.length, 'folders_popin.folders_one', 'folders_popin.folders_n'),
    totalFichiers > 0 ? nb(totalFichiers, 'folders_popin.files_one', 'folders_popin.files_n') : '',
    formatSize(totalTaille),
  ].filter(Boolean).join(' · '));

  const maintenant = Date.now();
</script>

<svelte:window onkeydown={(e) => e.key === 'Escape' && (open = false)} />

<!-- Porté vers `body` : ouverte depuis la barre latérale, elle y restait enfermée
     (son conteneur porte une transformation, qui piège les `fixed`). -->
<!-- `sidebar` ne sert plus qu'aux polices (Figtree, mono) ; couleurs = jetons --rg. -->
<div use:portal>
<div class="sidebar fixed inset-0 z-9990 flex items-center justify-center p-4">
  <button
    type="button"
    class="absolute inset-0 bg-black/60 backdrop-blur-sm cursor-default"
    onclick={() => open = false}
    aria-label={$t('common.close')}
  ></button>

  <div
    role="dialog"
    aria-modal="true"
    aria-labelledby="dossiers-titre"
    class="relative w-full max-w-xl max-h-[80vh] flex flex-col overflow-hidden rounded-2xl text-(--rg-tx)
           bg-(--rg-carte) border border-(--rg-bd) dark:border-[#2a312d]
           shadow-[0_24px_60px_-12px_rgba(0,0,0,0.25)] dark:shadow-[0_30px_80px_-20px_rgba(0,0,0,0.9)]"
  >
    <!-- En-tête -->
    <div class="shrink-0 flex items-center gap-3 px-6 pt-5 pb-4 border-b border-(--rg-bd)
                bg-linear-135 from-(--rg-g)/8 to-transparent to-60%">
      <span class="w-10 h-10 rounded-xl shrink-0 flex items-center justify-center
                   bg-(--rg-gbg) border border-(--rg-gbd) text-(--rg-g)">
        <Icon icon="material-symbols:folder-managed-outline-rounded" width="21" class="sb-icone" />
      </span>
      <div class="flex-1 min-w-0">
        <h2 id="dossiers-titre" class="text-[17px] font-extrabold tracking-[-0.01em] leading-tight text-(--rg-tx)">{$t('folders_popin.title')}</h2>
        <p class="mt-0.5 text-xs text-(--rg-mu) truncate">{loading ? $t('common.loading') : resume}</p>
      </div>
      <button
        type="button"
        class="-mr-1.5 w-8 h-8 shrink-0 rounded-[9px] flex items-center justify-center cursor-pointer transition-colors
               text-(--rg-mu) hover:bg-(--rg-hover) hover:text-(--rg-tx)"
        onclick={() => open = false}
        title={$t('common.close')}
        aria-label={$t('common.close')}
      >
        <Icon icon="material-symbols:close-rounded" width="18" />
      </button>
    </div>

    <!-- Dossiers -->
    <div class="flex-1 overflow-y-auto px-6 py-4 smart-scroll">
      {#if loading}
        <div class="flex items-center justify-center py-12">
          <Icon icon="lucide:loader-2" width="20" class="animate-spin text-(--rg-mu)" />
        </div>

      {:else if dirs.length === 0}
        <div class="flex flex-col items-center justify-center py-10 text-center rounded-2xl
                    border border-dashed border-(--rg-bd2) bg-(--rg-creux)">
          <Icon icon="material-symbols:folder-open-outline-rounded" width="32" class="text-(--rg-mu2) mb-2" />
          <p class="text-sm font-semibold text-(--rg-tx2)">{$t('folders_popin.empty')}</p>
          <p class="text-xs text-(--rg-mu) mt-0.5">{$t('folders_popin.empty_desc')}</p>
        </div>

      {:else}
        <!-- Mode Dengar / Preset Folder -->
        <div class="mb-4 p-3.5 rounded-2xl bg-(--rg-creux) border border-(--rg-bd) flex flex-col gap-2.5">
          <div class="flex items-center justify-between gap-2">
            <div class="flex items-center gap-1.5 text-xs font-bold text-(--rg-tx2)">
              <Icon icon="material-symbols:tune-rounded" width="16" class="text-(--rg-g)" />
              <span>Mode Dengar / Preset Folder</span>
            </div>
            <button
              type="button"
              class="flex items-center gap-1 px-2.5 py-1 rounded-lg text-xs font-semibold cursor-pointer transition-colors
                     {isCreatingPreset ? 'bg-(--rg-s2) text-(--rg-tx)' : 'text-(--rg-g) hover:bg-(--rg-gbg)'}"
              onclick={() => { isCreatingPreset = !isCreatingPreset; }}
            >
              <Icon icon={isCreatingPreset ? "material-symbols:close-rounded" : "material-symbols:bookmark-add-outline-rounded"} width="14" />
              <span>{isCreatingPreset ? 'Batal' : '+ Simpan Preset'}</span>
            </button>
          </div>

          {#if isCreatingPreset}
            <form
              class="flex items-center gap-2 pt-1"
              onsubmit={(e) => { e.preventDefault(); handleSavePreset(); }}
            >
              <input
                type="text"
                class="flex-1 px-3 py-1.5 text-xs rounded-xl bg-(--rg-carte) border border-(--rg-bd) text-(--rg-tx) placeholder:text-(--rg-mu) focus:outline-none focus:border-(--rg-g)"
                placeholder="Nama preset (cth. Pop & Fokus, Tanpa Orchestra...)"
                bind:value={newPresetName}
              />
              <button
                type="submit"
                disabled={!newPresetName.trim()}
                class="px-3 py-1.5 rounded-xl text-xs font-bold bg-(--rg-g) text-(--rg-on-g) disabled:opacity-40 cursor-pointer disabled:cursor-default transition hover:brightness-110"
              >
                Simpan
              </button>
            </form>
          {/if}

          <!-- Preset chips row -->
          <div class="flex flex-wrap items-center gap-1.5 pt-0.5">
            <!-- Built-in chip: Semua Aktif -->
            <button
              type="button"
              disabled={applyingBatch}
              class="flex items-center gap-1.5 px-3 py-1.5 rounded-xl text-xs font-semibold cursor-pointer transition-all border
                     {allActive
                       ? 'bg-(--rg-gbg) text-(--rg-g) border-(--rg-gbd) shadow-xs'
                       : 'bg-(--rg-carte) text-(--rg-mu) border-(--rg-bd) hover:text-(--rg-tx) hover:border-(--rg-bd2)'}"
              onclick={() => handleApplyBatch(dirs.map(d => d.id))}
              title="Aktifkan semua folder"
            >
              <Icon icon="material-symbols:select-all-rounded" width="14" />
              <span>Semua Aktif</span>
            </button>

            <!-- Custom profiles -->
            {#each profiles as profile (profile.id)}
              {@const active = isProfileActive(profile)}
              <div
                class="group inline-flex items-center rounded-xl text-xs font-semibold transition-all border
                       {active
                         ? 'bg-(--rg-gbg) text-(--rg-g) border-(--rg-gbd) shadow-xs'
                         : 'bg-(--rg-carte) text-(--rg-tx2) border-(--rg-bd) hover:border-(--rg-bd2)'}"
              >
                <button
                  type="button"
                  disabled={applyingBatch}
                  class="flex items-center gap-1.5 pl-3 py-1.5 pr-1.5 cursor-pointer text-left"
                  onclick={() => handleApplyBatch(profile.activeDirIds)}
                  title="{profile.activeDirIds.length} folder aktif: {profile.name}"
                >
                  <Icon icon="material-symbols:bookmark-outline-rounded" width="14" />
                  <span>{profile.name}</span>
                  <span class="text-[10px] opacity-70">({profile.activeDirIds.length})</span>
                </button>
                <button
                  type="button"
                  class="w-6 h-6 mr-1 rounded-lg flex items-center justify-center cursor-pointer transition-colors
                         opacity-60 hover:opacity-100 hover:bg-black/10 dark:hover:bg-white/10"
                  title="Hapus preset {profile.name}"
                  aria-label="Hapus preset {profile.name}"
                  onclick={(e) => handleDeletePreset(e, profile.id)}
                >
                  <Icon icon="material-symbols:close-rounded" width="13" />
                </button>
              </div>
            {/each}
          </div>
        </div>

        <div class="flex flex-col gap-2">
          {#each dirs as dir (dir.id)}
            {@const suivi = importIci && imp.directory === dir.path}
            {@const enCours = suivi || rescanningId === dir.id}
            <div class="flex items-center gap-3 p-3 rounded-2xl bg-(--rg-creux) border border-(--rg-bd)
                        transition-all hover:border-(--rg-bd2) {dir.is_active ? '' : 'opacity-55'}">
              <span class="w-10 h-10 rounded-xl shrink-0 flex items-center justify-center
                           {dir.is_active ? 'bg-(--rg-gbg) border border-(--rg-gbd) text-(--rg-g)' : 'bg-(--rg-s2) border border-(--rg-bd) text-(--rg-mu)'}">
                <Icon icon={dir.is_active ? "material-symbols:folder-outline-rounded" : "material-symbols:folder-off-outline-rounded"} width="20" class="sb-icone" />
              </span>

              <div class="flex-1 min-w-0">
                <p class="text-sm font-bold truncate text-(--rg-tx)">{dir.name}</p>
                <p class="font-mono text-[11px] truncate text-(--rg-mu2)" title={dir.path}>{dir.path}</p>

                <div class="flex flex-wrap items-center gap-1.5 mt-2 text-[11px] font-medium">
                  {#if dir.total_files > 0}
                    <span class="px-2 py-0.5 rounded-md bg-(--rg-s2) text-(--rg-tx2)">
                      {nb(dir.total_files, 'folders_popin.files_one', 'folders_popin.files_n')}
                    </span>
                  {/if}
                  {#if dir.total_size > 0}
                    <span class="px-2 py-0.5 rounded-md bg-(--rg-s2) text-(--rg-tx2)">{formatSize(dir.total_size)}</span>
                  {/if}
                  {#if dir.watch_enabled}
                    <span class="flex items-center gap-1 px-2 py-0.5 rounded-md bg-(--rg-gbg) text-(--rg-g)" title="Auto-Watch aktif (pantau file baru otomatis)">
                      <Icon icon="lucide:radar" width="12" />
                      Auto-Watch
                    </span>
                  {/if}
                  <span class="flex items-center gap-1.5 px-2 py-0.5 rounded-md bg-(--rg-s2) text-(--rg-tx2)">
                    <span class="sb-point w-1.5 h-1.5 rounded-full
                                 {enCours ? 'text-(--rg-am) animate-pulse' : dir.last_scan_at ? 'text-(--rg-g)' : 'text-(--rg-mu2)'}"></span>
                    {#if suivi && imp.stopping}
                      {$t('folders_popin.stopping')}
                    {:else if suivi && imp.phase === 'listing'}
                      {$t('folders_popin.listing').replace('{n}', imp.found.toLocaleString($currentLocale))}
                    {:else if suivi}
                      {imp.current.toLocaleString($currentLocale)} / {imp.total.toLocaleString($currentLocale)}
                    {:else if enCours}
                      {$t('folders_popin.scanning')}
                    {:else if dir.scan_status === 'cancelled'}
                      {$t('folders_popin.cancelled')}
                    {:else if dir.last_scan_at}
                      {$t('folders_popin.scanned').replace('{ago}', shortSince(dir.last_scan_at, maintenant, $t))}
                    {:else}
                      {$t('folders_popin.never_scanned')}
                    {/if}
                  </span>
                </div>

                {#if suivi && imp.phase === 'importing'}
                  <div class="mt-2 h-1 rounded-full overflow-hidden bg-(--rg-s2)">
                    <div class="h-full rounded-full bg-(--rg-g) transition-[width] duration-300" style="width: {imp.percent}%"></div>
                  </div>
                  <p class="mt-1 font-mono text-[10px] truncate text-(--rg-mu2)" title={imp.fileName}>{imp.fileName}</p>
                {/if}
              </div>

              <div class="flex items-center gap-1.5 shrink-0 self-start">
                <div class="mr-1 flex items-center" title={dir.is_active ? 'Included in library' : 'Excluded from library'}>
                  <ToggleSwitch
                    size="sm"
                    checked={dir.is_active}
                    disabled={enCours || imp.active}
                    label={dir.name}
                    onclick={() => handleToggleDirActive(dir)}
                  />
                </div>
                <button
                  type="button"
                  class="w-8 h-8 rounded-[9px] flex items-center justify-center cursor-pointer transition-colors
                         {dir.watch_enabled ? 'text-(--rg-g) bg-(--rg-gbg) border border-(--rg-gbd)' : 'text-(--rg-mu) hover:bg-(--rg-s2) hover:text-(--rg-tx)'}"
                  title={dir.watch_enabled ? 'Auto-Watch aktif (pantau file baru otomatis)' : 'Aktifkan Auto-Watch folder ini'}
                  aria-label={dir.watch_enabled ? 'Auto-Watch aktif' : 'Aktifkan Auto-Watch'}
                  onclick={() => handleToggleDirWatch(dir)}
                >
                  <Icon icon="lucide:radar" width="18" />
                </button>
                {#if suivi}
                  <button
                    type="button"
                    class="w-8 h-8 rounded-[9px] flex items-center justify-center cursor-pointer transition-colors
                           text-(--rg-am) enabled:hover:bg-(--rg-ambg) disabled:cursor-default disabled:opacity-50"
                    title={$t('folders_popin.stop')}
                    aria-label={$t('folders_popin.stop')}
                    disabled={imp.stopping}
                    onclick={() => importProgressStore.stop()}
                  >
                    <Icon icon="material-symbols:stop-circle-outline-rounded" width="18" />
                  </button>
                {:else}
                  <button
                    type="button"
                    class="w-8 h-8 rounded-[9px] flex items-center justify-center cursor-pointer transition-colors
                           text-(--rg-mu) enabled:hover:bg-(--rg-s2) enabled:hover:text-(--rg-tx) disabled:cursor-default disabled:opacity-50"
                    title={$t('folders_popin.rescan')}
                    aria-label={$t('folders_popin.rescan')}
                    disabled={enCours || imp.active}
                    onclick={() => handleRescanDir(dir)}
                  >
                    <Icon icon="material-symbols:refresh-rounded" width="18" class={enCours ? 'animate-spin text-(--rg-am)' : ''} />
                  </button>
                {/if}
                <button
                  type="button"
                  class="w-8 h-8 rounded-[9px] flex items-center justify-center cursor-pointer transition-colors
                         text-(--rg-mu) hover:bg-red-500/8 hover:text-red-600 dark:hover:text-red-400"
                  title={$t('folders_popin.remove')}
                  aria-label={$t('folders_popin.remove')}
                  onclick={() => handleRemoveDir(dir)}
                >
                  <Icon icon="material-symbols:delete-outline-rounded" width="18" />
                </button>
              </div>
            </div>
          {/each}
        </div>
      {/if}
    </div>

    <!-- Pied : même facture que PopinFooter -->
    <div class="shrink-0 flex items-center justify-between gap-3 px-6 py-3.5 border-t border-(--rg-bd) bg-(--rg-creux)">
      <button
        type="button"
        class="flex items-center gap-2 h-10 pl-4 pr-5 rounded-xl cursor-pointer text-sm font-bold transition
               bg-(--rg-g) text-(--rg-on-g) hover:brightness-110"
        onclick={handleAddFolder}
      >
        <Icon icon="material-symbols:add-rounded" width="18" />
        {$t('folders_popin.add')}
      </button>
      <button
        type="button"
        class="h-10 px-4 rounded-xl cursor-pointer text-sm font-semibold transition-colors
               text-(--rg-tx2) hover:bg-(--rg-hover) hover:text-(--rg-tx)"
        onclick={() => open = false}
      >
        {$t('common.close')}
      </button>
    </div>
  </div>
</div>
</div>
