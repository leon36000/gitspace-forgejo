# CONTINUATION-251 — preuve de barrière MCP Rust et source courante

Date : 27 octobre 2026. Worktree `webhook-005-workspace`, HEAD de base `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. Lot lecture seule; aucun service ni test n’a été relancé, aucune modification de production ou du code Forgejo.

## Réconciliation

Le runner egress direct CONT-250 exécute `scripts/run_mission_012_stdio_mcp_egress_gate_e2e.py`, dont le host MCP est une fixture barrière Python et dont l’observateur parent identifie l’adaptateur Python. Cette preuve ne déclare pas observer le host Rust.

Le chemin composé courant est distinct : `run_mission_012_live_composed_e2e.py` passe les marqueurs privés au stage agent; `mission_012_live_mcp_agent_e2e.py` choisit le `mission_012_sandboxed_test_adapter`; celui-ci ajoute les options au superviseur de confiance. À la trame `shutdown`, le superviseur écrit le PID, start-time et exécutable du véritable enfant `mission-012-mcp-host`, puis attend un reçu de gate à nonce avant de transmettre le shutdown. `verify_candidate_while_host_alive` vérifie PID/start-time, chemin et SHA-256 du binaire admis, teste l’arbre candidat en lecture seule, relit l’identité du processus et crée la libération.

Le parcours [CONTINUATION-245](../continuation-245-same-host-gate-001/local-run.md) a archivé `passed_while_admitted_mcp_host_remained_alive`; son vérificateur indépendant passe 2/2 tandis que le processus Rust exact est vivant. Le reçu déclare ensuite que le stage agent finalise et que le host est fermé avant le stage PR/merge.

Le script de cette revue lit le snapshot `composition_source_sha256` du reçu CONT-245 et le compare aux fichiers présents. Sur 114 entrées, 109 sont inchangées et cinq divergent : `scripts/run_forgejo_live_pr_merge_e2e.py`, `scripts/run_mission_012_stdio_mcp_egress_gate_e2e.py`, `scripts/run_webhook_reconcile_cli_e2e.py`, `tests/test_forgejo_live_pr_merge_e2e.py` et `tests/webhook_receiver_process_e2e.rs`. Les neuf fichiers centraux au cycle de vie de la barrière et du host Rust sont identiques au reçu; les hashes et les noms sont dans [`source-comparison.json`](source-comparison.json). Le snapshot complet de CONT-245 ne correspond donc pas intégralement au worktree courant.

## Portée

Cette comparaison confirme la preuve de présence du host Rust pendant le gate pour les sources centrales identiques; elle ne crée pas un nouveau résultat d’exécution. `n=1`, adaptateur déterministe, aucun LLM/AgentOS. Le host n’est pas attesté pendant la fusion; la politique du superviseur n’est pas indépendante et l’exclusion des writers extérieurs n’est pas prouvée. Le hash du binaire Forgejo ne révèle pas son commit de build. Cela ne démontre pas de séparation de privilèges, de garantie contre un noyau compromis ou un gain comparatif. Après finalisation du rapport, une seconde entrée append-only réindexe les versions finales de trois fichiers ajustés (rapport, hashes de sources et log de contrôle), sans modifier le premier index ni ses préfixes; les deux reçus de préfixe sont conservés.
