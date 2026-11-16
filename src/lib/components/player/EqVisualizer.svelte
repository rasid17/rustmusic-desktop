<script lang="ts">
  import { onDestroy } from "svelte";

  let {
    active = false,
    class: className = "",
  }: {
    active?: boolean;
    class?: string;
  } = $props();

  const PHASES = [0.0, 1.2, 2.4, 0.8, 1.8];
  const FREQS = [1.6, 2.0, 1.4, 1.8, 1.7];

  let heights = $state<number[]>([2, 2, 2, 2, 2]);
  let rafId = 0;
  let startTs = 0;

  function stopLoop() {
    if (rafId) {
      cancelAnimationFrame(rafId);
      rafId = 0;
    }
  }

  function tick(now: number) {
    if (!active) {
      rafId = 0;
      return;
    }
    const t = (now - startTs) / 1000;
    heights = PHASES.map((phase, i) => {
      const wave = (Math.sin(t * FREQS[i] * Math.PI * 2 + phase) + 1) * 0.5;
      return 2.4 + wave * 9.6; // 2.4px to 12px
    });
    rafId = requestAnimationFrame(tick);
  }

  $effect(() => {
    if (active) {
      if (!rafId) {
        startTs = performance.now();
        rafId = requestAnimationFrame(tick);
      }
    } else {
      stopLoop();
      heights = [2, 2, 2, 2, 2];
    }
  });

  onDestroy(stopLoop);
</script>

<div
  class="inline-flex items-center justify-center gap-[2.5px] h-3.5 {className}"
  aria-hidden="true"
>
  {#each heights as h, i (i)}
    <span
      class="w-[2.2px] rounded-full bg-(--lc-acc) {active ? '' : 'transition-[height] duration-420 ease-out'}"
      style:height="{h.toFixed(1)}px"
    ></span>
  {/each}
</div>
