import { invoke } from "@tauri-apps/api/core";
import { convertFileSrc } from "@tauri-apps/api/core";

export type CoverSize = '1x' | '2x' | 'full';

// Le protocole d'assets de Tauri ne sert pas les fichiers à la WebView de dev
// sous WebKitGTK (asset:// comme http://asset.localhost échouent). En dev, on
// passe par des data: URLs ; le build de production garde le protocole rapide.
const COVER_MODE: 'asset' | 'base64' = import.meta.env.DEV ? 'base64' : 'asset';

/** convertFileSrc encode les / en %2F sur Linux → 403. On les décode.
 *  Le double slash après `localhost` est voulu : Tauri y lit un chemin
 *  absolu (`/home/...`) et le fait correspondre au scope `$HOME/**`.
 *  Le supprimer donne `home/...`, que le protocole d'assets refuse. */
function assetSrc(path: string): string {
    return convertFileSrc(path)
        .replaceAll('%2F', '/');
}

function resolvedPathFor(path: string, size: CoverSize): string {
    return path.replace(/[\/\\]full[\/\\]/, `/${size}/`);
}

// ── URL finale gardée en mémoire ──
// Sans elle, chaque remontage de carte redemandait la miniature au backend ;
// la zone restait vide quelques images, puis l'image apparaissait (le « clignotement »).
const MAX_ASSETS = 3000;
const assets = new Map<string, string>();

function assetGet(path: string): string | undefined {
    const connu = assets.get(path);
    if (connu === undefined) return undefined;
    assets.delete(path);
    assets.set(path, connu);
    return connu;
}

function assetSet(path: string, url: string) {
    if (assets.size >= MAX_ASSETS) assets.delete(assets.keys().next().value!);
    assets.set(path, url);
}

/** URL déjà connue, sans attendre le backend. `null` = pas de couverture, `undefined` = inconnue. */
export function coverSrcCached(
    path: string | null | undefined,
    size: CoverSize = 'full',
): string | null | undefined {
    if (!path) return null;

    const resolvedPath = resolvedPathFor(path, size);
    if (resolvedPath.startsWith("http") || resolvedPath.startsWith("data:") || resolvedPath.startsWith("blob:")) return resolvedPath;
    if (path.startsWith("/images/")) return path;

    if (COVER_MODE === 'asset') {
        const connu = assetGet(resolvedPath);
        if (connu !== undefined) return connu;
        // La grande pochette ne dépend d'aucune génération : son URL est
        // connue tout de suite. Attendre l'observateur laissait le hero vide.
        if (size === 'full') {
            const url = assetSrc(resolvedPath);
            assetSet(resolvedPath, url);
            return url;
        }
        return undefined;
    }
    return lruGet(resolvedPath);
}

// ── Mode base64 (fallback) ──
const MAX_CACHE = 300;
const pending = new Map<string, Promise<string>>();
const resolved = new Map<string, string>();

function lruGet(path: string): string | undefined {
    if (!resolved.has(path)) return undefined;
    const val = resolved.get(path)!;
    resolved.delete(path);
    resolved.set(path, val);
    return val;
}

function lruSet(path: string, val: string) {
    if (resolved.size >= MAX_CACHE) {
        resolved.delete(resolved.keys().next().value!);
    }
    resolved.set(path, val);
}

// ── Miniatures : une seule question au backend par chemin ──
const MAX_MINIATURES = 2000;
const miniatures = new Map<string, Promise<string>>();

function miniature(chemin: string, original: string): Promise<string> {
    const deja = assetGet(chemin);
    if (deja) return Promise.resolve(deja);

    const connue = miniatures.get(chemin);
    if (connue) {
        miniatures.delete(chemin);
        miniatures.set(chemin, connue);
        return connue;
    }
    const p = invoke<string>("resolve_cover_thumbnail", { path: chemin })
        .then((reel) => {
            const url = assetSrc(reel);
            assetSet(chemin, url);
            miniatures.delete(chemin);
            return url;
        })
        .catch(() => {
            miniatures.delete(chemin);
            const url = assetSrc(original);
            assetSet(chemin, url);
            return url;
        });
    if (miniatures.size >= MAX_MINIATURES) miniatures.delete(miniatures.keys().next().value!);
    miniatures.set(chemin, p);
    return p;
}

// ── Fonction publique ──
// ── Pochettes floutées (fonds) : générées une fois par le backend ──
const MAX_FLOUES = 500;
const floues = new Map<string, Promise<string | null>>();

/** Version pré-floutée et saturée d'une pochette du disque ; `null` si impossible (distante, intégrée). */
export function resolveBlurredCover(
    path: string | null | undefined,
    saturation: number,
    flou: number,
): Promise<string | null> {
    if (!path || path.startsWith("http") || path.startsWith("data:") || path.startsWith("blob:") || path.startsWith("/images/")) {
        return Promise.resolve(null);
    }
    const cle = `${path}|${saturation}|${flou}`;
    const connue = floues.get(cle);
    if (connue) return connue;
    const p = invoke<string>("resolve_blurred_cover", { path, saturation, flou })
        .then(async (chemin) => {
            if (!chemin) return null;
            if (import.meta.env.DEV) {
                return await invoke<string>("read_cover_as_base64", { path: chemin });
            }
            return assetSrc(chemin);
        })
        .catch(() => {
            floues.delete(cle);
            return null;
        });
    if (floues.size >= MAX_FLOUES) floues.delete(floues.keys().next().value!);
    floues.set(cle, p);
    return p;
}

export async function resolveCoverSrc(path: string | null | undefined, size: CoverSize = 'full'): Promise<string | null> {
    if (!path) return null;

    const resolvedPath = resolvedPathFor(path, size);

    if (resolvedPath.startsWith("http") || resolvedPath.startsWith("data:") || resolvedPath.startsWith("blob:")) return resolvedPath;
    // Image de l'interface (« pas de CD ») : servie telle quelle, ce n'est pas un fichier du disque.
    if (path.startsWith("/images/")) return path;

    // Mode asset protocol (rapide)
    if (COVER_MODE === 'asset') {
        // Miniature (1x/2x) générée à la volée ; la grande en cas d'échec.
        if (size !== 'full') return miniature(resolvedPath, path);
        const deja = assetGet(resolvedPath);
        if (deja) return deja;
        const url = assetSrc(resolvedPath);
        assetSet(resolvedPath, url);
        return url;
    }

    // Mode base64 (fallback)
    const cached = lruGet(resolvedPath);
    if (cached) return cached;

    if (!pending.has(resolvedPath)) {
        const promise = invoke<string>("read_cover_as_base64", { path: resolvedPath })
            .then(dataUri => {
                lruSet(resolvedPath, dataUri);
                pending.delete(resolvedPath);
                return dataUri;
            })
            .catch(error => {
                pending.delete(resolvedPath);
                throw error;
            });
        pending.set(resolvedPath, promise);
    }
    return pending.get(resolvedPath)!;
}
