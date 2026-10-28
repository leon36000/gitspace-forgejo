# CONTINUATION-410 — un readback Release HTTP 403 reste inconclusif

Date : 2026-10-28. Worktree : `local/WEBHOOK-005-canonical-links`; HEAD observé : `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`.

## Parcours et résultat

La recherche ciblée n’a trouvé dans `tests/webhook_receiver_process_e2e.rs` que des refus 403 processus génériques pour Issue; aucun n’exerçait Release. Le scénario `actual_reconcile_cli_process_keeps_release_forbidden_read_inconclusive_without_projection` passe par le vrai receiver local, admet une livraison signée `release.deleted` (ID 613) à la séquence 1, puis lance le vrai binaire `forgejo-reconcile-local` contre une fixture loopback qui répond HTTP 403 au GET exact `/api/v1/repos/{owner}/{repo}/releases/613`.

Le CLI termine en échec et produit `verdict=inconclusive`, `error.code=upstream_forbidden`, `retryable=false`, avec l’action « inspect repository visibility and local API permissions ». L’erreur reste liée au kind Release, owner, repository et ID. L’attestation du journal est vérifiée à la seule portée `receiver_signature_only`. Il n’y a pas de `canonical_state`; la réponse publique ne porte pas Authorization; le corps 403 et les marqueurs de l’événement restent absents de stdout/stderr; l’inbox demeure octet-identique après le readback.

Commande : `run_webhook_reconcile_cli_e2e.py --scenario actual_reconcile_cli_process_keeps_release_forbidden_read_inconclusive_without_projection`. Le runner compile hors ligne et exécute le test sous Bubblewrap `--unshare-net --unshare-pid`. Résultat : **1 passed, 0 failed, 0 ignored, 196 filtered out**, code de sortie 0. Le log complet est `process-test.log`.

## Vérifications et portée

`cargo fmt --all -- --check`, `git diff --check`, l’AST du runner, la garde de licence Cargo, Cargo metadata et les manifestes append-only passent. Cargo reste `license=null`, `license_file=null`, `publish=[]`, métadonnées `no_license_selected`, `redistribution_rights=not_assessed`, `NOASSERTION`. Aucun choix de licence n’est fait.

Le lot modifie le test processus et sa liste de scénarios seulement; aucune logique runtime ni dépendance ne change. La réponse API est synthétique et loopback. Aucun Forgejo live, modèle, AgentOS, PR ou merge n’est utilisé. L’action opérable conseille de vérifier visibilité et permissions; la fixture ne vérifie pas que cette action résout réellement l’accès sur Forgejo.
