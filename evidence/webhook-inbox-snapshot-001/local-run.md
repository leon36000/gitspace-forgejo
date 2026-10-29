# WEBHOOK-INBOX-SNAPSHOT-001 — export local du journal, sans restauration

Date : 28 octobre 2026. Worktree local : `/home/pc6/Documents/Codex/2026-10-21-mission-construire-une-forge-con-ue/.runtime/webhook-005-workspace`, base observée `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`, branche `local/WEBHOOK-005-canonical-links`. Le checkout était déjà très modifié; aucun reset, commit, push, appel GitLab, lecture de courriel ou changement de service externe n’a eu lieu dans ce lot.

## Changement

`EventJournal::snapshot_read_only` prend le verrou latéral exclusif, lit une fois un journal privé borné à 64 Mio, vérifie que chaque ligne est terminée, que séquence et hash précédent suivent la chaîne, que le hash d’événement est correct et que les clés d’idempotence sont uniques. Il retourne les événements validés et les mêmes octets source, sans réparer la queue ni modifier le journal. Les métadonnées du reçu sont calculées sur ces octets exacts.

`forgejo-webhook-local --snapshot-inbox <inbox.jsonl> <nouvelle-copie.jsonl>` est une commande locale sans écoute réseau. Elle exige un répertoire parent possédé par l’utilisateur en mode `0700`, refuse un fichier destination existant ou symlinké, crée le nouveau fichier avec `create_new`/`O_NOFOLLOW`, force le mode `0600`, puis synchronise le fichier et le répertoire parent. stdout expose le schéma `forgejo-webhook-inbox-snapshot-v1`, la taille, le nombre d’événements, la dernière séquence/hash et SHA-256; les charges utiles ne sont pas imprimées. Une erreur d’écriture signale qu’un fichier partiel peut exister et ne doit pas être restauré.

## Échec puis correction

La commande d’intégration a d’abord échoué avant l’implémentation : le parser traitait `--snapshot-inbox` comme une adresse d’écoute (`invalid listen address`). Après ajout de l’API et du mode CLI, les cinq scénarios du binaire passent. Le reçu positif est comparé aux octets exacts et au SHA-256 indépendant; les refus vérifient que la source ou une destination préexistante reste inchangée.

## Vérifications

- `cargo test --locked --offline --test webhook_inbox_snapshot --test webhook_inbox_capacity -- --test-threads=1` : 11 réussis (5 nouveaux snapshots, 6 capacité), aucun échec.
- `cargo test --locked --offline --all-targets -- --test-threads=1` : 771 réussis, 0 échec, 200 ignorés, 39 groupes sur le source final.
- `cargo fmt --all -- --check` : code 0.
- `git diff --check` : code 0.
- `python3 scripts/verify_cargo_license_state.py` : passe; `license=null`, `license_file=null`, `publish=[]`, aucun choix de licence, redistribution non évaluée, SPDX `NOASSERTION`.
- `cargo metadata --format-version 1 --locked --offline --no-deps` : code 0; aucune dépendance ajoutée.
- Audit des liens Markdown des cinq documents canoniques : résultat archivé dans `documentation-links.json`.

Les sorties brutes, commandes, versions et empreintes des sources sont adjointes à ce reçu. `all-targets-final.log` est le run large qui inclut la dernière correction de mode de fichier.

## Limites

Ce chemin fournit une copie cohérente seulement entre processus coopératifs qui prennent le même `flock` sur ce même hôte et système de fichiers. Un webhook peut être acquitté après l’instant de copie; restaurer cette copie seule peut donc perdre une livraison confirmée. La fonctionnalité n’implémente ni quiescence, ni restauration, ni replay, ni migration, compaction, rotation ou rétention. Elle ne sauvegarde pas la configuration, les secrets, le dépôt Git, la base Forgejo ou les lockfiles latéraux. Un SHA-256 lie des octets; il ne prouve ni leur authenticité ni la correction des événements. Un écrivain hostile du même UID peut contourner le verrou. Les 200 tests ignorés sont hors de cette exécution générale et gardent leurs exigences d’environnement documentées.

## Suite

Définir puis éprouver un protocole de restauration vérifiée avec quiescence ou replay/réconciliation, y compris la reprise après interruption et la détection d’un fichier snapshot incomplet. Ne pas présenter cet export comme mécanisme de sauvegarde produit. Le parcours LLM/AgentOS réutilisable sans dépense et le benchmark comparatif apparié restent à démontrer.
