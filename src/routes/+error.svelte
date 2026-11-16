<script lang="ts">
  import { page } from "$app/state";
  import { goto } from "$app/navigation";
  import Icon from "@iconify/svelte";
</script>

<div class="flex flex-1 flex-col items-center justify-center p-8 text-center h-full">
  <div class="w-16 h-16 mb-4 rounded-2xl border flex items-center justify-center bg-red-500/10 border-red-500/20 text-red-500">
    <Icon icon="material-symbols:error-outline-rounded" width="32" />
  </div>
  <h2 class="text-lg font-bold text-(--rg-tx) mb-1">
    {page.status} — {page.error?.message ?? "Terjadi kesalahan"}
  </h2>
  <p class="text-xs text-(--rg-mu) max-w-md mb-5">
    Halaman tidak dapat dimuat secara penuh. Silakan kembali ke halaman utama atau muat ulang tampilan.
  </p>
  <div class="flex items-center gap-2.5">
    <button
      type="button"
      onclick={() => goto("/")}
      class="px-4 py-2 rounded-xl text-xs font-semibold bg-(--rg-g) text-(--rg-on-g) hover:brightness-110 cursor-pointer transition"
    >
      Ke Beranda
    </button>
    <button
      type="button"
      onclick={() => window.location.reload()}
      class="px-4 py-2 rounded-xl text-xs font-semibold border border-(--rg-bd2) text-(--rg-tx) hover:bg-white/5 cursor-pointer transition"
    >
      Muat Ulang
    </button>
  </div>
</div>
