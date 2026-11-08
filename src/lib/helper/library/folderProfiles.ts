import { invoke } from "@tauri-apps/api/core";
import { forgetLocations } from "#lib/helper/library/trackLocation";
import { libraryContentStore } from "#lib/stores/library/libraryContent.store";
import { libraryStore } from "#lib/stores/library/library.store";
import { importsTermines } from "#lib/stores/library/importProgress.store";

export type FolderProfile = {
  id: string;
  name: string;
  activeDirIds: string[];
};

/**
 * Retrieves the saved folder profiles (listening modes) for a given library from localStorage.
 */
export function getFolderProfiles(libraryId: number): FolderProfile[] {
  try {
    const raw = localStorage.getItem(`rustmusic:folder_profiles:${libraryId}`);
    if (!raw) return [];
    const parsed = JSON.parse(raw);
    return Array.isArray(parsed) ? parsed : [];
  } catch (e) {
    console.error("Failed to read folder profiles:", e);
    return [];
  }
}

/**
 * Persists the folder profiles for a given library to localStorage.
 */
export function saveFolderProfiles(libraryId: number, profiles: FolderProfile[]): void {
  try {
    localStorage.setItem(`rustmusic:folder_profiles:${libraryId}`, JSON.stringify(profiles));
  } catch (e) {
    console.error("Failed to save folder profiles:", e);
  }
}

/**
 * Applies a folder profile by updating active dirs in batch on backend and refreshing stores.
 */
export async function applyFolderProfile(libraryId: number, activeDirIds: string[]): Promise<void> {
  await invoke("set_library_dirs_active_batch", { libraryId, activeDirIds });
  forgetLocations();
  await Promise.all([
    libraryContentStore.refresh(libraryId),
    libraryStore.refresh(),
  ]);
  importsTermines.update((n) => n + 1);
}
