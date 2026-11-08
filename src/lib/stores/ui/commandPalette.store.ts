import { writable } from "svelte/store";

function createCommandPaletteStore() {
  const { subscribe, set, update } = writable({ open: false, initialQuery: "" });
  return {
    subscribe,
    open: (initialQuery = "") => set({ open: true, initialQuery }),
    close: () => set({ open: false, initialQuery: "" }),
    toggle: () => update((s) => ({ open: !s.open, initialQuery: "" })),
  };
}

export const commandPaletteStore = createCommandPaletteStore();
