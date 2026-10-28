# CONTINUATION-020-SUPERVISOR-CLEANUP-STATUS-001

Date : 26 octobre 2026. Produit local : `agent-forge-core`, worktree `local/WEBHOOK-005-canonical-links`, HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. Le worktree était déjà fortement modifié; aucun changement préexistant n'a été réinitialisé ni commité.

## Constat et changement

Avant ce lot, `kill_verifier_scope` signalait `False` si `cgroup.kill` et le fallback D-Bus étaient tous deux refusés, mais `terminate_host_group` ignorait ce retour. L'arrêt forcé du host finissait donc avec le code 75, comme lorsque chaque demande de kill avait été acceptée.

Le test de processus avec échec injecté a d'abord reçu 75 au lieu du code attendu 76 : [sortie rouge](red-before-fix.log). `terminate_host_group` renvoie maintenant si toutes les demandes de kill de scope ont été acceptées. Le processus superviseur retourne 76 si au moins l'une est refusée, après avoir quand même tenté SIGTERM puis SIGKILL sur le groupe du host. Le code 75 demeure l'issue d'un arrêt forcé lorsque toutes les demandes ont été acceptées ou qu'aucun scope n'a été relevé.

La distinction est volontairement limitée : les deux codes signalent un arrêt forcé. Le 75 ne prouve pas que tous les processus sont morts; cgroup.kill ou systemd peut avoir accepté une requête sans readback d'un cgroup vide. Le 76 indique qu'une demande n'a pas été acceptée et que l'arrêt du scope n'est pas confirmé. Le test intégré normal de cancellation fait séparément le readback du scope et observe qu'il est vide.

## Vérification

- `python3 tests/test_mission_012_mcp_supervisor.py` : 15 tests réussis, 0 échec. Inclut le refus unitaire agrégé tout en terminant le groupe du host, et un vrai processus superviseur qui renvoie 76 après injection d'un retour `False`; le host enfant est absent après la sortie.
- `python3 tests/test_mission_012_sandboxed_test_adapter.py -k stdio_adapter_uses_only_trusted_mcp_pipe_and_refuses_host_network_and_file` : 1 réussi. Vérifie que le chemin normal de l'adaptateur reste cohérent; ce test n'injecte pas le nouveau code 76 dans un adaptateur.
- `cargo test --locked --offline --test mission_012_mcp_lifecycle mcp_supervisor_cancellation_terminates_the_in_flight_verifier_scope -- --exact --nocapture` : 1 réussi avec le vrai host et le scope cgroup de vérification.
- `python3 -m py_compile ...`, `cargo fmt --all -- --check`, `git diff --check` et `python3 scripts/verify_cargo_license_state.py` réussissent; les sorties sont archivées dans ce dossier.
- Cargo confirme `license=null`, `license_file=null`, `publish=[]`, `license_decision=no_license_selected`, `redistribution_rights=not_assessed`; aucun choix de licence n'est fait.

Commandes, versions, sorties et SHA-256 des sources sont archivés dans les fichiers voisins; `SHA256SUMS` couvre les pièces de ce run. Les manifestes produit et parent restent append-only et sont vérifiés par `update_manifests.py`.

## Limites et suite

Le code 76 est la sortie du superviseur MCP et ne crée pas de réponse JSON-RPC après la fermeture forcée du serveur. `MCPClient.close` retourne le statut; `mission_012_test_agent_adapter.py::run` transforme tout statut non nul en exception et l'entrée `main` la rend comme événement `error`. Ce raccord est établi par lecture du code, mais le run de ce lot ne fait pas traverser le code 76 à ce client fixture et ne prouve pas la façon dont chaque client MCP externe expose l'état. Le refus de cleanup est injecté au niveau de fonction; il ne force pas réellement le noyau et systemd à refuser tous deux un cgroup actif. Le cas réellement observé du fallback systemd accepté se trouve dans [CONTINUATION-019](../continuation-019-cgroup-kill-fallback/local-run.md) et n'a pas été rejoué ici. Portée de système : Linux aarch64, systemd 255 et cgroup v2; autres environnements non établis.

Aucun Forgejo live, modèle, AgentOS, coût, secret, service externe, publication ou production. Projet global incomplet : l'intégration LLM et la comparaison appariée restent non démontrées. Prochaine tranche locale indépendante : reprendre l'audit ciblé des routes d'écriture Forgejo et des effets sur la récupération LFS sans relancer les POC déjà archivés; avant d'étendre la sémantique de statut, tester la propagation d'une fermeture MCP anormale à travers les adaptateurs locaux.
