# CONTINUATION-409 — `release.deleted` puis état courant encore présent

Date : 2026-10-28. Worktree : `local/WEBHOOK-005-canonical-links`; HEAD observé : `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`.

## Parcours et résultat

Le scénario `actual_reconcile_cli_process_keeps_deleted_release_present_when_current_api_returns_200` passe par le vrai receiver local, qui admet une enveloppe signée `release.deleted` sous une clé HMAC de test et la journalise à la séquence 1. Le vrai binaire `forgejo-reconcile-local` lit ensuite l’ID 612 sur l’endpoint repository-scoped exact. La fixture API renvoie 200 et le Release courant `v1.4.4`, tandis que l’événement porte `stale-deleted-tag`.

Le CLI retourne `current_state_read`, `upstream_read_attempts=1` et `canonical_state.state=present`, avec le tag et l’URL de l’API courante. La source et la fraîcheur sont `current_forgejo_resource_api` et `current_api_read`; le contenu reste `untrusted_user_content`. Notes/assets et marqueurs privés de l’événement et de l’API sont absents. Le GET n’a pas d’en-tête Authorization et l’inbox est octet-identique après lecture. L’action `deleted` n’est donc pas projetée comme état si l’API dit que la ressource existe encore.

Commande : `run_webhook_reconcile_cli_e2e.py --scenario actual_reconcile_cli_process_keeps_deleted_release_present_when_current_api_returns_200`. Le runner compile hors ligne puis exécute le binaire du test sous Bubblewrap `--unshare-net --unshare-pid`; sa fixture HTTP loopback est la seule route de test. Résultat final : **1 passed, 0 failed, 0 ignored, 195 filtered out**, code de sortie 0. Voir `process-test.log`.

## Diagnostic initial et vérifications

Le premier run rouge du scénario a échoué uniquement dans l’assertion de test : elle consultait `journal_source.receiver_attestation.scope`, qui vaut null. Le sérialiseur du CLI et les scénarios voisins utilisent le champ `verification_scope` (`src/bin/forgejo-reconcile-local.rs`, sérialisation de l’attestation); la correction de cette assertion fait passer le scénario. Ce n’était pas un échec du comportement de réconciliation. Un contrôle de format initial a aussi signalé deux ajustements rustfmt; `cargo fmt --all` les a appliqués au test, puis `cargo fmt --all -- --check` et `git diff --check` passent. Le log rouge et les sorties initiales sont conservés.

`verify_cargo_license_state.py` confirme `metadata_policy_verified`. `cargo metadata --offline --locked --no-deps` garde `license=null`, `license_file=null`, `publish=[]` et les métadonnées `no_license_selected`, droits `not_assessed`, `NOASSERTION`. Aucune licence n’est sélectionnée. Hashes, toolchain et manifestes sont enregistrés dans ce dossier.

## Limites

Test processus local, HMAC et serveur API synthétiques. Il établit la réaction du client à un readback HTTP 200; il ne prouve pas qu’un webhook natif Forgejo a été émis ni l’état réel d’un Forgejo vivant. Aucun modèle/AgentOS, PR, décision de fusion ou merge n’est exercé. Aucune logique runtime ou dépendance ne change dans ce lot; aucune opération distante n’a été effectuée.
