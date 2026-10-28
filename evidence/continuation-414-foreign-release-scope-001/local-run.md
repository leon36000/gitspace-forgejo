# CONTINUATION-414 — Release étranger refusé avant credential et API

Date : 2026-10-28. Worktree : `local/WEBHOOK-005-canonical-links`; HEAD de base : `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`.

## Frontière testée

Le receiver n’a pas de configuration owner/repository dans son interface HTTP. Le test confirme donc le contrat d’inbox partagée : une livraison `release.deleted` valide, signée par les fixtures HMAC et receiver locales, pour `outside-org/unconfigured-release-repository` reçoit HTTP 202 et est durablement journalisée. Ce n’est pas un refus d’ingress.

Le vrai `forgejo-reconcile-local`, configuré pour `forge-admin/agent-forge-core`, relit ensuite cette livraison avec son attestation receiver vérifiée. Le verdict est `inconclusive/resource_out_of_scope`, non réessayable, avec l’action de router vers l’adaptateur du dépôt correspondant. `upstream_read_attempts=0`, aucun `canonical_state`; le fixture API reçoit zéro requête. Le pipe de credential hôte synthétique garde tous ses octets et le token n’apparaît pas dans stdout/stderr. Le marqueur de corps du webhook ne fuit pas et le journal reste octet-identique.

Cette frontière évite qu’un adaptateur de dépôt transmette son credential ou fasse une lecture API pour un autre dépôt. La redirection automatique vers le bon adaptateur n’est pas démontrée ni implémentée dans ce lot.

## Commande et résultat

Le runner `scripts/run_webhook_reconcile_cli_e2e.py --scenario actual_reconcile_cli_process_refuses_foreign_release_before_credential_or_api` compile offline/locked puis lance le scénario sous Bubblewrap `--unshare-net --unshare-pid`. La commande exacte est dans `commands.txt`; le log complet est `process-test.log`.

Résultat : **1 passed, 0 failed, 0 ignored, 200 filtered out**, code de sortie 0. `cargo fmt --all -- --check`, `git diff --check`, l’AST du runner, la garde de licence et `cargo metadata --offline --locked --no-deps --format-version 1` passent. Cargo conserve `license=null`, `license_file=null`, `publish=[]`, `no_license_selected`, droits `not_assessed`, `NOASSERTION`.

## Portée et limites

Aucune logique runtime ou dépendance ne change; le lot ajoute un scénario processus, sa liste de runner et précise la documentation d’architecture. Le test utilise un receiver signé, une clé d’attestation et un credential synthétiques, plus un fixture HTTP loopback. `receiver_signature_only` prouve la signature locale du journal, pas l’origine Forgejo. Aucun Forgejo live, vrai token, LLM/AgentOS, PR/fusion ni benchmark n’est exercé; la suite complète n’est pas relancée.
