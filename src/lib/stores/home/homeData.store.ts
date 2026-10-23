import type { AlbumListView } from "#lib/types/ui/library/album/AlbumListView";
import type { GenreMix } from "#lib/types/ui/library/genre/GenreMix";
import type { GenreView } from "#lib/types/ui/library/genre/GenreView";
import type { LibraryStats } from "#lib/types/ui/library/stats/LibraryStats";

export type HomeData = {
  stats: LibraryStats | null;
  genres: GenreView[];
  mixGenres: GenreMix[];
  albums: AlbumListView[];
};

const empty = (): HomeData => ({ stats: null, genres: [], mixGenres: [], albums: [] });
const cache = new Map<number, HomeData>();

/** Dernière photo connue d'une bibliothèque, pour repeindre l'accueil sans attendre le backend. */
export const homeDataCache = {
  get(libraryId: number): HomeData | undefined {
    return cache.get(libraryId);
  },
  set(libraryId: number, patch: Partial<HomeData>): void {
    cache.set(libraryId, { ...(cache.get(libraryId) ?? empty()), ...patch });
  },
  clear(libraryId?: number): void {
    if (libraryId == null) cache.clear();
    else cache.delete(libraryId);
  },
};
