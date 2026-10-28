# CONTINUATION-022-MCP-SANDBOX-STATUS-PROPAGATION

Date : 26 octobre 2026. Produit : `agent-forge-core`, worktree `/home/pc6/Documents/Codex/2026-10-21-mission-construire-une-forge-con-ue/.runtime/webhook-005-workspace`, branche `local/WEBHOOK-005-canonical-links`, HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. Le worktree était déjà fortement modifié; aucune modification préexistante n'a été supprimée, réinitialisée ou commise.

## Constat

Le premier run du nouveau test complet a échoué après environ 20 secondes avec `trusted MCP host did not finalize after adapter EOF`; le test n'a pas vu `exit 76`. Le wrapper a dû tuer le processus superviseur parce que celui-ci ne se terminait pas. La cause observée dans `scripts/mission_012_mcp_supervisor.py` était que la fermeture de stdin du host à EOF ne définissait pas `shutdown_deadline`. Seul SIGTERM démarrait ce délai, alors que le wrapper attendait déjà sa propre limite.

## Changement

`close_host_input` démarre maintenant `HOST_FINALIZATION_TIMEOUT_SECONDS` chaque fois qu'il ferme stdin, y compris après EOF. Un host qui ignore EOF est donc terminé après un délai borné; le superviseur conserve la distinction entre la demande de cleanup acceptée (75) et refusée (76).

Le wrapper `mission_012_sandboxed_test_adapter.py` transforme un code 76 du processus host en refus actionnable : échec de confirmation du cleanup du scope, exit 76, résultat candidat inconclusif. Les codes non nuls génériques incluent aussi leur valeur. Le chemin de délai du wrapper ferme son socket local s'il doit arrêter le processus après expiration.

## Test complet et preuve rouge/verte

`tests/test_mission_012_sandboxed_test_adapter.py::test_supervisor_cleanup_failure_survives_full_mcp_sandbox_path` lance le vrai wrapper, Bubblewrap et la fonction `main` du vrai code superviseur. Le shim du test importe le script de production, fixe le délai à 0,15 seconde et injecte uniquement `verifier_scope_paths` avec un chemin de scope synthétique sous l'UID courant et `kill_verifier_scope=False`. Le refus n'est pas une erreur réellement imposée par le noyau, `cgroup.kill` ou systemd.

Le host MCP fixture répond au contexte, au test de base rouge, aux lectures de fichiers, à l'écriture et au test corrigé vert, puis reste vivant après EOF. L'adaptateur stdio a alors déjà produit `success`; le vrai code superviseur ferme le host, retourne 76 à cause du refus injecté, et le wrapper hôte remplace ce succès par `type=error`, code de sortie 1. Les assertions vérifient le message `exit 76`, `verifier-scope cleanup`, `inconclusive`, l'absence de subtype `success`, les deux appels test rouge/vert et l'absence du PID host après l'arrêt.

Le run rouge établit le comportement antérieur à la correction : l'attente externe expirait à 20 secondes sans propagation du statut. Le run ciblé après correction passe en 0,276 s; le log est conservé dans `focused-e2e.log`.

## Vérifications

- `python3 tests/test_mission_012_sandboxed_test_adapter.py` : 6/6, log `python-sandbox-adapter.log`.
- `python3 tests/test_mission_012_mcp_supervisor.py` : 15/15, log `python-supervisor.log`.
- `python3 tests/test_mission_012_test_agent_adapter.py` : 1/1, log `python-agent-adapter.log`.
- `cargo test --locked --offline --test mission_012_mcp_lifecycle mcp_supervisor_cancellation_terminates_the_in_flight_verifier_scope -- --exact --nocapture` : 1/1, log `rust-cancellation-process.log`.
- Compilation syntaxique Python des deux scripts modifiés et du test, puis `git diff --check` : réussis.
- Python 3.12.3, Cargo/Rust 1.95.0, Bubblewrap 0.9.0, systemd 255.4, unified cgroup v2, Linux 7.0.0 aarch64.
- `git diff --check` and a separate trailing-whitespace check over the untracked source/test files and updated docs both pass.

## Portée et suite

Cette preuve traverse le processus Bubblewrap et le vrai code du superviseur, mais utilise un host, une mission, des résultats de test et un scope de fixtures. Elle ne prouve pas un refus réel de `cgroup.kill` avec fallback systemd, un workspace candidat compilé, une intégration AgentOS/LLM, une instance Forgejo ou un gain comparatif. Aucun LLM, Forgejo live, secret utilisateur, service extérieur, coût, publication ou production. Aucun code Cargo/Rust produit ou licence ne change dans ce lot.

Prochaine action : reprendre l'audit ciblé des writers Forgejo et de la récupération LFS uniquement avec une hypothèse distincte des POC déjà enregistrés; sinon, sélectionner un cas d'adaptateur/reconciliation non couvert. Garder le benchmark apparié suspendu tant qu'un runtime ou plafond fournisseur fiable n'est pas disponible. Le projet reste incomplet.
