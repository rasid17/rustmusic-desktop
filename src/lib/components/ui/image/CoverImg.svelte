<script lang="ts">
import { untrack } from "svelte";
import { coverSrcCached, resolveCoverSrc, type CoverSize } from "#lib/helper/tools/coverHelper";
import { whenNearViewport } from "#lib/helper/tools/nearViewport";
import FadeImg from "./FadeImg.svelte";
import type { HTMLImgAttributes } from "svelte/elements";

let { path, alt = "", size = 'full', ...rest }: {
    path: string | null | undefined;
    alt?: string;
    size?: CoverSize;
} & Omit<HTMLImgAttributes, "src" | "alt"> = $props();

// Lecture volontairement non réactive : c'est l'état initial de la carte.
const cached0 = untrack(() => coverSrcCached(path, size));
let src = $state<string | null>(cached0 ?? null);

// La miniature n'est demandée qu'à l'approche de l'écran : une grille de 900 cartes
// lançait 900 demandes au backend d'un coup (et autant de miniatures à fabriquer).
// Une URL déjà connue s'affiche tout de suite, sans attendre ce observateur.
let near = $state(cached0 !== undefined);
let anchor = $state<HTMLElement | null>(null);
$effect(() => {
    const currentPath = path;
    const cached = coverSrcCached(currentPath, size);
    if (cached !== undefined) {
        src = cached;
        near = true;
        return;
    }
    src = null;
    near = false;
    if (!currentPath || !anchor) return;
    return whenNearViewport(anchor, () => (near = true));
});

$effect(() => {
    const currentPath = path;
    if (!currentPath || !near) return;
    const cached = coverSrcCached(currentPath, size);
    if (cached !== undefined) {
        src = cached;
        return;
    }
    // Pochette changée avant la réponse : l'ancienne ne doit pas s'afficher.
    let vivant = true;
    resolveCoverSrc(currentPath, size).then(url => {
        if (vivant) src = url;
    });
    return () => { vivant = false; };
});
</script>

{#if src}
    <FadeImg {src} {alt} {...rest} />
{:else}
    <!-- Même place que l'image à venir : c'est elle qu'on surveille. -->
    <span bind:this={anchor} class="block {rest.class ?? ''}" aria-hidden="true"></span>
{/if}
