# CONTINUATION-279 — probe credential fermé en cas d’erreur inconnue

Date : 27 octobre 2026  
Worktree : `.runtime/webhook-005-workspace`  
HEAD de base : `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0` (worktree fortement modifié; aucun commit créé).

## Audit et changement

Le code courant de `scripts/mission_012_test_agent_adapter.py::probe_sandbox_capabilities` convertissait tout `OSError` du probe `credential_path.read_bytes()` en `credential_visible=false`. Le test TDD injectait `EIO`; avant la correction, il échouait parce qu’aucune erreur inconclusive n’était émise. Le probe accepte maintenant seulement `ENOENT`, `EACCES`, `EPERM` et `ENOTDIR`; toute autre erreur donne `RuntimeError("synthetic credential probe failed inconclusively")`. Les erreurs d’environnement inattendues ne deviennent donc pas une preuve d’inaccessibilité.

Le test de sonde vivant crée un memfd hôte avec l’unique chaîne synthétique `SYNTHETIC-GATE-CREDENTIAL-CANARY-ONLY`. Il confirme positivement que le FD reste lisible par le parent hôte, pointe le probe vers `/proc/<pid-hôte>/fd/<fd>`, puis garde Bubblewrap vivant durant les événements `ready`, `reprobe` et `release`. Le processus du probe rapporte `credential_visible=false`; les autres assertions vérifient ses FDs `[0,1,2]`, son namespace réseau distinct, les refus des trois routes ciblées et l’inaccessibilité/absence d’écriture pour les fichiers canaris sélectionnés. Aucun contenu de credential réel n’est utilisé.

Le module sandbox plus le validateur des preuves egress passe **29/29**. Le test `EIO` et le test processus memfd passent individuellement 1/1; `py_compile`, l’analyse des espaces finaux et `git diff --check` passent. Journaux et sources épinglées dans ce dossier.

## Revue des chemins et de l’autorité

Dans `scripts/mission_012_sandboxed_test_adapter.py::bubblewrap_command`, le root du sandbox ne bind-monte pas le workspace ou le `/` hôte. Il expose `/usr` et `/etc/alternatives` en lecture seule, l’adaptateur en lecture seule, et construit des tmpfs sur `/tmp`, `/home` et `/run`, avec `/dev` et `/proc`. Les probes de fichiers privés sont des tentatives limitées aux chemins canaris fournis, pas un inventaire exhaustif de tous les fichiers accessibles.

Dans `scripts/run_mission_012_live_agent_pr_e2e.py::run_rust_merge_gate`, le runner crée un pipe anonyme pour la credential et un socket de capacité; le token n’est pas placé dans les arguments. `pass_fds` fournit le FD credential et les canaux de capacité au seul exécutable Rust gate lancé via un shim. `scripts/mission_012_mcp_supervisor.py::main` démarre son host avec `close_fds=True` et aucun `pass_fds`. Ceci est une lecture statique des sources courantes, pas un test de l’autorisation réelle de fusion dans ce lot.

Les quatre fichiers archivés de `evidence/forgejo-agent-egress-stdio-mcp-006/source/` ont des hashes différents des versions courantes; cette exécution historique n’est donc pas réutilisée comme preuve du code courant.

## Limites

La pause est celle de la sonde Bubblewrap, pas celle d’une opération MCP de fusion. L’identité Unix locale est partagée. Le processus évalué est `local-test-adapter`, pas un LLM/AgentOS; aucune instance Forgejo, fusion, credential réel ou service externe n’a été utilisé. Les probes concernent les chemins explicitement sélectionnés et ne prouvent pas l’absence de toute voie de lecture, la sécurité face au même UID/root/noyau, ni une politique OS de production. Les refus fonctionnels cumulatifs, l’interface d’exécution AgentOS et le benchmark restent ouverts.
