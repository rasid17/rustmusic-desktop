-- Les « sans paroles » marqués avant le repli lyrics.ovh et le débit tenu
-- n'étaient parfois que des rejets 429 déguisés en absence : on les efface
-- pour que la nouvelle cascade les retente. Les vraies trouvailles
-- (lrclib, sidecar, manuel) restent en cache.
DELETE FROM lyrics WHERE source = 'none';
