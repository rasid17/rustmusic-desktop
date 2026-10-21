-- ============================================================================
-- Auto-mixes
-- ============================================================================
--
-- Un mix tire au hasard chaque jour, mais peut resservir les mêmes cartes deux
-- jours de suite et ne connaît ni la diversité ni la fraicheur. Un auto-mix
-- s'appuie sur les mêmes règles, courbe le tirage (plafond par artiste,
-- équilibre favoris ↔ redécouverte), se souvient de ses trois dernières
-- générations pour ne pas se répéter — et, surtout, MATÉRIALISE son résultat
-- dans `playlist_items` : il se lit, se met en file et se partage comme une
-- playlist ordinaire, un bouton le renouvelle à la demande.

-- Contenu calculé une fois puis stocké, à la différence des playlists
-- intelligentes qui se réévaluent à chaque ouverture.
ALTER TABLE playlists ADD COLUMN is_auto INTEGER NOT NULL DEFAULT 0;

-- JSON : { "longueur": 50, "par_artiste": 2, "fraicheur": 50 }
ALTER TABLE playlists ADD COLUMN mix_options TEXT NULL;

-- JSON : les trois dernières générations, listes d'identifiants de pistes.
-- Le tirage suivant les exclut — c'est ce qui rend le renouvellement visible.
ALTER TABLE playlists ADD COLUMN mix_recent TEXT NULL;
