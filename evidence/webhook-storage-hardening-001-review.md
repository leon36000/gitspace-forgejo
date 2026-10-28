# WEBHOOK-STORAGE-HARDENING-001 — revue indépendante

Revue en lecture seule de `src/lib.rs`, `src/bin/forgejo-webhook-local.rs` et `tests/webhook_receiver_process_e2e.rs`, après l’ajout du test sous umask `022`.

## Findings initiaux et résolution

1. **Création du journal trop permissive.** `EventJournal::append_locked` créait le journal sans mode explicite. Sous umask `022`, le vrai receiver écrivait l’inbox en `0644`; le prochain démarrage, qui exige `0600`, la refusait. Correction : `OpenOptionsExt::mode(0o600)`. Le test rouge/vert exerce le processus réel, la réception, le mode observé et le redémarrage.
2. **TOCTOU sur le secret.** L’ancienne lecture faisait `symlink_metadata(path)`, puis `fs::read(path)`, permettant au chemin de changer entre contrôle et lecture. Correction : ouverture unique avec `O_NOFOLLOW`, vérification des métadonnées du descripteur, puis lecture bornée du même descripteur; `O_NONBLOCK` empêche qu’un chemin FIFO ne bloque l’ouverture avant le contrôle du type.

## Verdict

Le correctif couvre les deux défauts dans le scénario examiné. La revue confirme que l’écriture du journal sous umask `022` produit le mode exigé et que la lecture du secret n’effectue plus de seconde résolution du chemin après les contrôles.

**Limite résiduelle :** `O_NOFOLLOW` protège le dernier composant du chemin, pas les composants parents. Le fichier finalement ouvert reste vérifié par type, UID, nombre de liens et permissions. Les processus du même UID restent dans la frontière de confiance locale.

La revue n’a apporté aucune modification et n’a pas exécuté de tests. Les preuves d’exécution sont dans `webhook-storage-hardening-001-local-run.md` et les journaux liés.
