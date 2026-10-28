# CONTINUATION-411 — refus du readback Release HTTP 401

Date : 2026-10-28. Worktree : `local/WEBHOOK-005-canonical-links`; HEAD de base : `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`.

## Parcours vérifié

Le test `actual_reconcile_cli_process_keeps_release_unauthorized_read_inconclusive_without_projection` démarre le vrai receiver local, admet une livraison `release.deleted` signée à la séquence 1, puis lance le vrai binaire `forgejo-reconcile-local` contre une fixture HTTP loopback répondant 401 au GET exact `/api/v1/repos/{owner}/{repo}/releases/614`.

Le CLI échoue avec un résultat structuré `verdict=inconclusive`, `error.code=upstream_unauthorized`, `retryable=false`, lié au kind Release, propriétaire, dépôt et ID. L’état canonique est absent. Aucun credential hôte n’est configuré et aucun header Authorization n’est envoyé. Les marqueurs synthétiques du webhook et du corps 401 sont absents de stdout/stderr; le journal du receiver reste octet-identique après le readback. L’attestation est limitée à `receiver_signature_only`.

## Commande et résultat

Le runner `scripts/run_webhook_reconcile_cli_e2e.py --scenario actual_reconcile_cli_process_keeps_release_unauthorized_read_inconclusive_without_projection` compile hors ligne, puis exécute le test sous Bubblewrap avec `--unshare-net --unshare-pid`. La commande exacte est archivée dans `commands.txt`; le transcript complet est dans `process-test.log`.

Résultat final : **1 passed, 0 failed, 0 ignored, 197 filtered out**, code de sortie 0. Le premier contrôle `cargo fmt --all -- --check` a identifié trois mises en forme dans ce test; ces lignes ont été corrigées sans reformater le dépôt, puis le test isolé a été rejoué et le contrôle de formatage est passé.

`cargo fmt --all -- --check`, `git diff --check`, l’analyse AST du runner, `python3 scripts/verify_cargo_license_state.py` et `cargo metadata --offline --locked --no-deps --format-version 1` passent. Les sorties pertinentes sont jointes. Cargo déclare toujours `license=null`, `license_file=null`, `publish=[]`, `no_license_selected`, droits `not_assessed`, `NOASSERTION`.

## Portée et limites

Le changement de code porte uniquement sur ce test processus et son entrée dans le runner; aucune logique runtime ni dépendance n’a changé. Le serveur API est une fixture synthétique loopback, pas une instance Forgejo live. Ce lot ne démontre ni l’ACL réelle, ni AgentOS/LLM, PR/fusion, benchmark ou amélioration générale.

En parallèle, un contrôle anonyme vers le remote GitLab PC4 échoue avant HTTP : `pc4.tail0c55ff.ts.net` ne se résout pas depuis PC6 et `git ls-remote` retourne 128. Aucun credential n’a été lu ou utilisé, aucune référence distante n’a été obtenue et aucun staging, commit ou push n’a été effectué. Les accès courriel antérieurement examinés n’ont pas confirmé de credential limité au projet cible; les valeurs ne sont pas conservées dans ce reçu.
