<script lang="ts">
  import { onDestroy } from "svelte";
  import { SpinClock } from "#lib/helper/player/spinClock";

  let {
    src,
    fallbackSrc = "/images/no-cd.png",
    spinning = false,
    size = 380,
    class: className = "",
  }: {
    src: string | null | undefined;
    fallbackSrc?: string;
    spinning?: boolean;
    size?: number;
    class?: string;
  } = $props();

  let angle = $state(0);
  let lastSrc = $state<string | null | undefined>(undefined);

  const clock = new SpinClock((a) => {
    angle = a;
  });

  $effect(() => {
    if (src !== lastSrc) {
      lastSrc = src;
    }
    clock.setRunning(spinning);
  });

  onDestroy(() => {
    clock.destroy();
  });
</script>

<div
  class="cd-disc-root relative shrink-0 rounded-full select-none pointer-events-none {className}"
  style:width="{size}px"
  style:height="{size}px"
>
  <!-- Punched disc assembly (13.5% center hole is truly transparent so the card shows through) -->
  <div class="cd-hole-punch relative w-full h-full rounded-full overflow-hidden">
    <!-- LAYER 1: Rotating disc body (artwork print + conic rainbow iridescence + outer rim) -->
    <div
      class="absolute inset-0 rounded-full will-change-transform"
      style:transform="rotate({angle}deg)"
    >
      <!-- Polycarbonate base tint -->
      <div class="absolute inset-0 rounded-full bg-[#16181d]"></div>

      <!-- Label print: full strength outside the 33% hub, thinned inside so bare plastic reads -->
      <div class="cd-label-mask absolute inset-0 rounded-full overflow-hidden">
        <img
          src={src || fallbackSrc}
          alt=""
          class="w-full h-full object-cover"
          onerror={(e) => ((e.currentTarget as HTMLImageElement).src = fallbackSrc)}
        />
      </div>

      <!-- Conic rainbow iridescence that turns WITH the disc -->
      <div class="cd-iridescence absolute inset-[1%] rounded-full opacity-30 mix-blend-screen"></div>

      <!-- Clear plastic outer rim & inner hairline -->
      <div class="absolute inset-[1%] rounded-full border-[2px] border-white/18"></div>
      <div class="absolute inset-[2%] rounded-full border border-black/28"></div>
    </div>

    <!-- LAYER 2: Stationary optical reflections (room light & polycarbonate hub rings) -->
    <div class="absolute inset-0 rounded-full">
      <!-- Diagonal specular light streak (stays stationary while disc turns underneath) -->
      <div class="cd-specular absolute inset-[1%] rounded-full"></div>

      <!-- Clear polycarbonate clamping hub (33% of disc diameter) -->
      <div
        class="cd-hub absolute top-1/2 left-1/2 -translate-x-1/2 -translate-y-1/2 w-[33%] h-[33%] rounded-full border border-white/45"
      ></div>

      <!-- Stacking ring / mirror band (22% of disc diameter) -->
      <div
        class="absolute top-1/2 left-1/2 -translate-x-1/2 -translate-y-1/2 w-[22%] h-[22%] rounded-full border border-white/24 bg-black/14"
      ></div>

      <!-- Center hole lip (14% of disc diameter, framing the 13.5% hole punch) -->
      <div
        class="absolute top-1/2 left-1/2 -translate-x-1/2 -translate-y-1/2 w-[14.2%] h-[14.2%] rounded-full border-[1.5px] border-black/65 shadow-[inset_0_0_0_1px_rgba(255,255,255,0.22)]"
      ></div>
    </div>
  </div>
</div>

<style>
  .cd-disc-root {
    filter: drop-shadow(0 14px 28px rgba(0, 0, 0, 0.52));
  }

  /* True transparent 13.5% center hole (radius 6.75% of box = 13.5% of radius) */
  .cd-hole-punch {
    -webkit-mask-image: radial-gradient(
      circle at center,
      transparent 0%,
      transparent 13.4%,
      #000 13.8%,
      #000 100%
    );
    mask-image: radial-gradient(
      circle at center,
      transparent 0%,
      transparent 13.4%,
      #000 13.8%,
      #000 100%
    );
  }

  /* Thins the artwork print inside the 33% hub to ~23% opacity so bare polycarbonate reads */
  .cd-label-mask {
    -webkit-mask-image: radial-gradient(
      circle at center,
      rgba(0, 0, 0, 0.23) 0%,
      rgba(0, 0, 0, 0.23) 32.5%,
      #000 33.5%,
      #000 98%,
      transparent 100%
    );
    mask-image: radial-gradient(
      circle at center,
      rgba(0, 0, 0, 0.23) 0%,
      rgba(0, 0, 0, 0.23) 32.5%,
      #000 33.5%,
      #000 98%,
      transparent 100%
    );
  }

  .cd-iridescence {
    background: conic-gradient(
      from 0deg,
      #ff9fd8 0deg,
      #9fe8ff 60deg,
      #fff29f 120deg,
      #c89fff 180deg,
      #9fffd0 240deg,
      #ffc49f 300deg,
      #ff9fd8 360deg
    );
  }

  .cd-specular {
    background: linear-gradient(
      135deg,
      rgba(255, 255, 255, 0.32) 0%,
      rgba(255, 255, 255, 0.12) 15%,
      transparent 36%,
      transparent 64%,
      rgba(255, 255, 255, 0.09) 85%,
      rgba(255, 255, 255, 0.2) 100%
    );
  }

  .cd-hub {
    background: radial-gradient(
      circle at center,
      rgba(255, 255, 255, 0.24) 0%,
      rgba(255, 255, 255, 0.13) 72%,
      rgba(255, 255, 255, 0.06) 100%
    );
  }
</style>
