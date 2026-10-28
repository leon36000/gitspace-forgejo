# WEBHOOK-FORK-PR-HOST-CREDENTIAL-001 — lecture de PR privée depuis un fork

Date : 25 octobre 2026 (heure locale). Worktree `.runtime/webhook-005-workspace`, HEAD de base `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`; le worktree contient de nombreux changements antérieurs non commités, donc ce HEAD ne désigne pas un snapshot propre des sources testées.

## Résultat

Le scénario `actual_reconcile_cli_process_reads_fork_pull_request_with_host_credential_fd` passe **1/1** lors du run initial et après correction de la revue (71 autres tests filtrés à chaque fois). Le run initial est archivé dans `process-e2e.log`; le nouveau run après correction P2 est archivé dans `review-fix-process-e2e.log`, avec code 0. Il relie le vrai récepteur HMAC, l’inbox durable et le vrai processus `forgejo-reconcile-local`. L’événement signé vise `forge-admin/agent-forge-core`, avec une base dans le dépôt cible et une tête dans `contributor/agent-forge-core`. Le processus du récepteur est arrêté avant la réconciliation; seul le CLI reçoit le credential synthétique sur son FD hôte.

Le refus avec credential invalide `actual_reconcile_cli_process_refuses_fork_pull_request_with_invalid_host_credential` passe **1/1** sous le runner Bubblewrap, 72 tests filtrés, code 0; le compte rendu est dans `invalid-host-credential-result.md`. Le refus sans FD `actual_reconcile_cli_process_keeps_fork_pull_request_inconclusive_without_host_credential` passe **1/1**, 73 filtrés, code 0; voir `no-host-credential-result.md`. Dans ce second cas, le CLI émet un GET anonyme vers la route cible et la fixture d’API privée le refuse en 401. Les deux tests exigent un seul GET, `inconclusive/upstream_unauthorized`, aucune donnée `canonical_state`, aucune fuite de token et une inbox byte-à-byte inchangée. Le refus sans FD est donc une réponse à l’échec d’authentification API, pas un blocage préalable du client.

La revue fraîche a signalé que le premier fixture authentifié se fermait après sa première connexion et ne pouvait pas attester l’absence d’une seconde requête. Le fixture corrigé reste à l’écoute jusqu’à la fin du CLI, draine les connexions déjà en attente et retourne chaque requête capturée; l’assertion `requests.len() == 1` prouve maintenant directement l’unicité observée au niveau socket, sans dépendre du compteur interne du CLI. Le test ciblé a été relancé après ce correctif : 1/1, code 0; sortie brute dans `review-fix-process-e2e.log`.

Le serveur HTTP loopback renvoie HTTP 200 seulement lorsqu’il reçoit le header `Authorization: token …` attendu. Le test constate exactement un `GET /api/v1/repos/forge-admin/agent-forge-core/pulls/45`, un readback `current_state_read` et les SHA de tête/base présents dans la réponse API, tous deux différents des valeurs périmées du webhook. Le token n’apparaît pas dans stdout ou stderr; la lecture ne modifie pas les octets de l’inbox.

Cela comble la composition entre la lecture d’une PR de fork et le passage d’un credential hôte : les deux comportements existaient séparément. La fixture refuse implicitement un appel sans le credential attendu en répondant 401; la réussite du CLI et la réponse authentifiée sont assertées. Aucun code runtime produit n’a été modifié.

## Vérifications

- Runner ciblé Bubblewrap : 1/1, `--unshare-net --unshare-pid`; dépendances Cargo hors ligne et verrouillées.
- Scénario de refus credential invalide sous Bubblewrap : 1/1, code 0; résultat détaillé dans `invalid-host-credential-result.md`.
- Scénario sans descripteur de credential sous Bubblewrap : 1/1, code 0; résultat détaillé dans `no-host-credential-result.md`.
- `rustfmt --edition 2024 --check tests/webhook_receiver_process_e2e.rs` : code 0.
- Compilation syntaxique Python du runner : code 0.
- `git diff --check` : code 0; un contrôle Python distinct vérifie les espaces de fin dans les trois fichiers touchés.
- Tests de garde Cargo : 7/7.
- Readback `cargo metadata`: `license=null`, `license_file=null`, `publish=[]`; garde : `no_license_selected`, droits `not_assessed`.

Les sorties brutes, codes de vérification, commandes et versions sont conservés dans ce dossier. Les SHA-256 des sources du worktree utilisées par cette tranche sont dans `source-hashes.txt`. Le HEAD indiqué est seulement le point de départ : le worktree est fortement modifié, donc ces empreintes ne constituent pas un diff ou une attribution Git propre. Le manifeste `SHA256SUMS` couvre les pièces locales.

## Portée et limites

Le serveur API est une fixture loopback, et le token est synthétique. Le run ne teste ni une instance Forgejo vivante, ni des ACL/scope réels, ni l’envoi natif du webhook par Forgejo, ni une fusion, ni un LLM/AgentOS, ni le vérificateur ou la valeur comparative. L’isolation réseau s’applique au runner de test; elle ne démontre pas l’isolation OS d’un futur agent. La suite Cargo `--all-targets` n’a pas été relancée : le résultat de ce dossier ne revendique que la compilation de la cible et le scénario ciblé.

Cargo conserve explicitement la licence produit non choisie et la publication désactivée; aucune décision juridique ou droit de redistribution n’est inféré. Projet global incomplet.
