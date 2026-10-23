<script lang="ts">
  import type { HTMLImgAttributes } from "svelte/elements";
  let { src, alt = "", class: classe = "", ...rest }: { src: string; alt?: string } & Omit<HTMLImgAttributes, "src" | "alt"> = $props();
  let errored = $state(false);
</script>

<!-- Pas de fondu : au retour sur une page, l'image est déjà connue et doit apparaître
     immédiatement. Une image introuvable reste masquée plutôt que d'afficher l'icône cassée. -->
<img
  {...rest}
  {src}
  {alt}
  loading="lazy"
  onerror={() => errored = true}
  class="{classe} {errored ? 'opacity-0' : ''}"
/>
