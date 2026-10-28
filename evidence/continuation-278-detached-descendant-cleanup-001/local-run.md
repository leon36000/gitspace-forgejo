# CONTINUATION-278 — nettoyage d’un descendant Bubblewrap détaché

Date : 27 octobre 2026  
Worktree : `.runtime/webhook-005-workspace`  
HEAD de base : `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0` (worktree fortement modifié; aucun commit créé).

## Résultat

Le test construit la commande à partir de `bubblewrap_command` avec ses namespaces, montages, environnement et options actuels, puis remplace uniquement l’entrée du `local-test-adapter` par un canari qui fork et appelle `setsid()` dans l’enfant. L’observation hôte `/proc` voit trois descendants; l’enfant détaché a une session/groupe distincts de la cible signalée directement par le parent. `stop_process_group` termine le lanceur Bubblewrap avec `SIGTERM` (code `-15`). Après nettoyage, aucun des identifiants PID/start-time observés ne subsiste et le stderr est vide.

Le probe indépendant reproductible et son relevé sont `run_detached_child_probe.py` et `process-cleanup-observation.json`. Le test de non-régression `test_process_group_cleanup_kills_detached_bubblewrap_descendant` passe 1/1; le module `tests.test_mission_012_sandboxed_test_adapter` passe **13/13**, 0 échec. La syntaxe Python, le contrôle ciblé des espaces finaux et `git diff --check` passent.

Le code existant de nettoyage n’a pas été modifié; ce lot ajoute seulement le test et un probe reproductible. Le résultat confirme le comportement de cette fixture locale et de ses options Bubblewrap sous arrêt commandé du wrapper.

Si un échec laisse un PID observé vivant, le `finally` de test n’envoie son signal de secours qu’après ouverture d’un pidfd et confirmation du start-time, pour éviter de signaler un processus réutilisant le même numéro.

## Portée et limites

Le canari ne contient ni donnée utilisateur ni credential réel; il ne contacte pas Forgejo. Cette preuve ne démontre pas le nettoyage d’un scope systemd de production, la terminaison forcée du parent hôte, la résistance à un processus de même UID/root ou au noyau, ni l’absence de descendants créés après l’observation. Aucun serveur Forgejo, LLM, AgentOS, nouvelle PR ou fusion n’a été lancé. Ce test ne remplace pas l’audit restant des accès directs aux fichiers et matériaux du gate; la mission demeure incomplète.

Toolchain locale : Python 3.12.3, Bubblewrap 0.9.0, Linux 7.0.0-1019-nvidia aarch64. Les commandes et hashes sont consignés dans ce dossier; les manifestes append-only sont vérifiés par `verify_manifests.py`.
