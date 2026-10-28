# FORGEJO-RECONCILE-SCOPE-PROCESS-001 — credentials retenus hors du profil configuré

Date : 25 octobre 2026. Worktree `local/WEBHOOK-005-canonical-links`, HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. Aucun code produit n’a été modifié dans ce lot; les deux tests existaient et étaient ignorés par défaut en attendant un namespace réseau privé.

## Comportements vérifiés

Les tests démarrent le vrai receiver et le vrai CLI Rust avec deux types d’inbox synthétique signée, puis un serveur HTTP local de fixture.

- Dans un batch mixte, le CLI lit l’issue `41` de `forge-admin/agent-forge-core` une seule fois via le serveur de fixture. L’événement `42` de `other-owner/other-repository` termine `resource_out_of_scope`; aucune requête ne vise son dépôt. Le credential de fixture transite par pipe privé et n’apparaît pas dans stdout/stderr.
- Pour un batch entièrement hors profil, le verdict reste `inconclusive`, `upstream_read_attempts=0`, les deux lignes sont `resource_out_of_scope` et le pipe credential reste intégralement non consommé. Aucun détail d’identité de ressource n’est émis pour la seconde ligne.

Ces parcours exercent les binaires et un serveur HTTP de test dans un namespace Bubblewrap `--unshare-net`; ils n’appellent pas Forgejo. Les payloads et le credential sont des fixtures synthétiques. L’inbox est lue par le CLI; l’API est représentée par le serveur loopback que le test lance dans le même namespace.

## Exécution

Les deux tests normalement ignorés ont été sélectionnés individuellement avec `--ignored --exact` et ont chacun terminé `1 passed, 0 failed` :

- `actual_reconcile_cli_process_mixed_batch_limits_credential_to_configured_repository`
- `actual_reconcile_cli_process_denied_batch_does_not_read_host_credential_fd`

Les sorties intégrales sont `bwrap-test.log` et `denied-batch-bwrap-test.log`; les commandes intégrales et versions sont dans `commands.txt` et `toolchain.txt`. Bubblewrap monte le système en lecture seule, `/tmp` en tmpfs privé, le seul `target/` de Cargo en écriture et le registre Cargo en lecture seule. Cargo est offline et verrouillé. La durée maximale de chaque commande était bornée à 90 secondes.

Le garde Cargo confirme `license=null`, `license_file=null`, `publish=[]`, `no_license_selected`, droits `not_assessed`; aucune licence n’est sélectionnée. Aucun LLM, AgentOS, RAG, mémoire, service externe, donnée privée, déploiement ou coût n’est utilisé.

## Limites

Ces tests établissent le filtrage de profil et l’usage du credential pour ces deux cas de processus. Ils n’exercent pas l’authentification contre un Forgejo live, un credential Forgejo réel, SSH, d’autres profils, multi-hôte ou un processus hostile de même UID. Le payload émis est un fixture, pas un événement natif. Cette preuve ne constitue pas une garantie générale de sécurité du système.
