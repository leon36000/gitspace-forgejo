# CONTINUATION-249 — reprise après `SIGKILL` pendant `Retry-After`

Date : 27 octobre 2026. Worktree préexistant `.runtime/webhook-005-workspace`, HEAD de base `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. Aucun commit, publication, changement de production ou appel fournisseur.

## Scénario

Le helper `run_retry_after_interruption_and_resume(signal)` alimente le vrai receiver Rust avec une livraison d’issue HMAC-signée, arrête le receiver après append durable, puis démarre le vrai CLI `forgejo-reconcile-local`. Une fixture HTTP loopback répond `429 Retry-After: 5`. Après réception complète et 200 ms, le test confirme que le CLI attend toujours et qu’aucune deuxième requête n’est arrivée. Il envoie le signal au groupe du CLI, vérifie l’arrêt dans les trois secondes et l’absence de sortie partielle, puis relit les octets de l’inbox.

Deux tests utilisent ce même scénario : `interrupted_reconcile_during_retry_after_resumes_from_unchanged_journal` envoie `SIGTERM` (CONT-248, retesté après factorisation); `killed_reconcile_during_retry_after_resumes_from_unchanged_journal` envoie `SIGKILL` (nouveau cas CONT-249). Dans les deux cas, il n’y a eu qu’un GET et le journal contient une livraison, octet pour octet identique. Le CLI est ensuite relancé sur le même journal et le même delivery ID; une fixture `200 open` donne `current_state_read`, un GET et toujours le journal inchangé.

## Vérifications

Les deux scénarios passent séparément 1/1 (153 filtrés chacun) par `scripts/run_webhook_reconcile_cli_e2e.py`. Le runner compile `webhook_receiver_process_e2e` avec Cargo `--offline --locked --no-run`, puis exécute chaque test explicitement dans Bubblewrap `--unshare-net --unshare-pid`. Journaux complets : `sigterm.log` et `sigkill.log`; codes de sortie 0. `cargo fmt --all -- --check`, `git diff --check`, le readback `python3 scripts/verify_cargo_license_state.py` et les 12 tests `scripts.test_cargo_license_state` passent. La suite Rust complète n’a pas été relancée.

Cargo conserve `license=null`, `license_file=null`, `publish=[]`, décision `no_license_selected`, droits de redistribution `not_assessed`, export SPDX `NOASSERTION`; aucune licence n’est attribuée.

## Réévaluation de l’interface AgentOS/runtime

Le checkout local OPSYS-AI est au commit `f0b89ccf86d6f6e50eb134de3257af91860e8091`, version Python `2.0.0.dev0`, sans modification de fichiers suivis au relevé. `src/opsys/serveur_mcp.py::creer_serveur` expose un serveur MCP (stdio et HTTP streamable), mais la forge ne montre toujours aucun adaptateur de cette interface vers `src/agent_runtime.rs::AgentRuntimeV1`; `tests/agent_runtime_contract.rs::FixtureRuntime` reste le seul adaptateur d’exécution repéré dans la forge. SHA-256 des sources OPSYS consultées : `serveur_mcp.py` `63a936da0cb46b6d81e545c7dcf69925447962ee06f72cf17216fabc0827b4e4`; `pyproject.toml` `52c28285c60781968a3182dc28414da8fb1e982dc3e821d7007142814e5fe5de`. Un inventaire passif de sockets trouve `127.0.0.1:9119` en écoute sans attribution d’identité; aucun trafic n’a été envoyé à ce listener. Aucun listener n’apparaît aux ports documentés OPSYS 8765/18789 ou Ollama 11434. Ce contrôle ne prouve pas l’absence d’autres services sur d’autres ports. Aucun modèle ni credential n’a été invoqué; le présent test est une fixture déterministe.

## Portée et limites

Linux local, un scénario par signal (`n=1` chacun), fixture HTTP loopback et arrêt forcé après 200 ms d’un délai de cinq secondes. Le test établit la conservation du journal pour cette interruption du processus; il ne couvre pas une panne d’alimentation, une panne/disparition du disque, un crash noyau, une reprise multi-hôte, une instance Forgejo réelle, le temps complet de retry, un LLM/AgentOS ou une amélioration de productivité. L’instance OPSYS MCP n’a pas été contactée; le listener 9119 reste non attribué.

Les empreintes de sources et de journaux sont dans `source-sha256.txt`; `append_manifests.py` ajoute les pièces et vérifie les préfixes append-only des manifestes local, worktree et projet.
