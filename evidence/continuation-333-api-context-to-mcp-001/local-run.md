# CONTINUATION-333 — lecture API canonique reliée au contexte MCP

Date : 28 octobre 2026. Worktree : `local/WEBHOOK-005-canonical-links`, HEAD de base `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`, fortement modifié. Ce lot ajoute un test de processus ciblé et étend la sélection du runner; le code d’exécution applicatif n’a pas changé.

## Parcours observé

`actual_cli_current_issue_labels_reach_mcp_context_from_signed_webhook` relie dans un même test le vrai processus receiver, le vrai CLI d’admission et le vrai processus `mission-012-mcp-host`. Un webhook HMAC synthétique contient un titre/corps anciens et le label périmé `{id:99,name:stale/webhook-label}`. La fixture HTTP loopback répond au GET API de l’issue avec le titre/corps courants et `{id:7,name:current/api-label}`. Le CLI inscrit le snapshot API dans le handoff; le processus MCP retourne exactement le même `untrusted_work_item`, marque sa source `current_forgejo_issue_api` et sa confiance `untrusted_user_content`, et conserve les exigences protégées inchangées. Le test compare directement le résultat au label du webhook pour vérifier l’exclusion de celui-ci.

Le processus hôte est ensuite arrêté sans preuve indépendante ni candidat modifié. Il sort avec le code 2 et écrit `host_finalization.outcome=refused`; la seule lecture de contexte ne clôt donc pas une mission.

## Commandes et résultats

- `python3 scripts/run_forgejo_mission_delivery_process_e2e.py --only actual_cli_current_issue_labels_reach_mcp_context_from_signed_webhook` — PASS, 1/1; 7 autres cas du binaire restent filtrés. Sortie complète : [context-only-e2e.log](context-only-e2e.log).
- `cargo fmt --all -- --check` — PASS : [rustfmt.log](rustfmt.log).
- `python3 scripts/verify_cargo_license_state.py` — PASS : `license=null`, `license_file=null`, `publish=[]`, `redistribution_rights=not_assessed`, `NOASSERTION`; [cargo-license-state.json](cargo-license-state.json).
- `git diff --check` — PASS pour les fichiers suivis : [tracked-diff-check.log](tracked-diff-check.log).
- Empreintes SHA-256 des sources modifiées : [source-hashes.txt](source-hashes.txt).

Le runner rapporte Rust/Cargo 1.95.0, Bubblewrap 0.9.0 et Linux aarch64; le test s’exécute avec espaces réseau et PID séparés et un `.runtime` temporaire privé. Le code des fixtures et du handoff est entièrement synthétique.

## Tentative composée non réussie et limites

Une exécution séparée du test `mission_012_signed_request_context_red_green_independent_verification_and_sandbox_merge` montre que le processus MCP retourne bien le contexte API courant et le label 7, mais s’arrête à son premier `cargo_test` avec `Failed to connect to bus: No medium found`. La sandbox n’a pas de bus systemd utilisateur; le red/green, la correction, la PR, la vérification et la fusion ne sont donc pas établis par cette relance. Le journal est conservé dans [process-e2e.log](process-e2e.log). Le test ciblé ci-dessus évite `cargo_test` et ne remplace pas cette preuve manquante.

Cette preuve relie le CLI au MCP mais emploie une fixture Forgejo loopback et aucun LLM/AgentOS. Elle ne démontre ni comportement Forgejo live, ni modification réelle du code, PR, vérification indépendante, fusion ou gain. Le refus de clôture est observé pour ce scénario seulement.
