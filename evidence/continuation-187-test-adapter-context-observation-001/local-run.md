# CONTINUATION-187 — preuve structurée de consommation du contexte issue

Date : 27 octobre 2026. Worktree `.runtime/webhook-005-workspace`, HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`, fortement modifié. Pas de changement de Cargo ou de licence, de commit, d’accès externe, de publication ni de production.

## Lacune, échec et correction

Le run CONTINUATION-186 avait un champ `untrusted_work_item` dans le DTO d’admission et un appel `mission.context`, mais le résumé ne conservait pas un signal distinct indiquant que le test-adapter avait validé ce champ. Trois nouvelles suites ont d’abord échoué : deux cas de résultat sans booléen, absence des validateurs du runner et du gate composé. Les logs rouges sont conservés dans `adapter-red-tests.log`, `agent-runner-red-tests.log` et `composition-red-tests.log`.

Le test-adapter expose maintenant `run_with_evidence`, qui retourne l’observation directe de `observe_untrusted_work_item`; l’événement JSON donne un booléen `test_adapter_untrusted_work_item_observed` et ne recopie aucun titre/corps. Le wrapper Bubblewrap refuse une valeur absente ou non booléenne et la transmet. Le runner compare le booléen à la présence du champ dans la requête d’admission. La validation composée propage cette présence depuis le DTO et exige l’égalité; une exécution Claude doit laisser ce champ de preuve à `null`.

## Vérifications

- `python3 tests/test_mission_012_test_agent_adapter.py` — 9/9.
- `python3 tests/test_mission_012_sandboxed_test_adapter.py` — 9/9. Le test de processus lance le vrai adaptateur via le wrapper Bubblewrap/MCP; l’hôte de fixture fournit une issue hostile marquée non fiable, le wrapper relaie `true`, et le résultat ne contient pas le texte hostile. Le chemin d’interruption après écriture relaie aussi `true`.
- `python3 tests/test_mission_012_live_mcp_agent_e2e.py` — 9/9; les réponses absentes, non booléennes ou différentes de l’admission sont refusées.
- `python3 tests/test_mission_012_live_composed_e2e.py` — 6/6; le gate refuse l’absence/la contradiction et un résultat modèle qui se présente comme preuve du test-adapter.
- `cargo test --offline --locked --features test-crash-injection --test mission_012_mcp_lifecycle sigkill_before_and_after_workspace_effect_are_recovered_under_a_new_fence -- --exact --nocapture` — 1/1, 31 filtrés. Le vrai host MCP Rust et le test-adapter s’exécutent ensemble sur une fixture locale avec deux SIGKILL (avant et après effet); à chaque reprise l’événement de succès porte le booléen vrai et le checkpoint `unverified` est repris.
- AST de huit sources Python, `cargo fmt --all -- --check`, `git diff --check`, `python3 scripts/verify_cargo_license_state.py`, et `cargo metadata --offline --locked --no-deps --format-version 1` passent. Les métadonnées restent `license=null`, `license_file=null`, `publish=[]`; aucune licence n’est choisie.

Commandes et résultats : `test-green-*.log`, `final-test-green-*.log`, `verified2-test-sandbox-wrapper.log`, `final-rust-mcp-lifecycle.log`, `verified2-rust-mcp-lifecycle.log` et `static-verification.log`. Les empreintes sources et versions sont dans `source-hashes.txt` et `toolchain.txt`.

## Portée et limites

La frontière booléenne est vérifiée par tests de composant et de processus, puis dans un parcours composé Forgejo frais après instrumentation. Ce parcours `n=1`, avec le `local-test-adapter`, est documenté et lié aux reçus dans [CONTINUATION-187-FULL](../mission-012-live-agent-webhook-cont187-context-observation/local-run.md); les pièces webhook/admission sont dans le [dossier admission](../mission-012-live-agent-webhook-cont187-context-observation-admission/local-run.md). Le host MCP est réel dans le banc, mais l’issue, le dépôt et le travail sont synthétiques. Aucun LLM ni AgentOS/RAG/mémoire réel n’est impliqué; le booléen confirme l’observation du DTO par l’adaptateur déterministe, pas ce qu’un modèle a réellement consommé. Les tests couvrent les chemins exercés, pas l’exclusion générale de tous writers ni la sécurité de production.

Le benchmark apparié conventionnel reste non fait; aucun gain, nombre d’interventions ou coût LLM n’est établi. La licence GitSpace et les droits de redistribution sont encore inconnus/non évalués. Les manifestes append-only enregistrent ce reçu et le checkpoint; leurs préfixes antérieurs sont liés dans `manifest-prefix-before.json`.
