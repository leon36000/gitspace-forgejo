# MISSION-012-TOOL-OPERATION-JOURNAL-001 — writer MCP à résultat durable

Date de vérification : 25 octobre 2026. Worktree : `/home/pc6/Documents/Codex/2026-10-21-mission-construire-une-forge-con-ue/.runtime/webhook-005-workspace`. HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`, branche `local/WEBHOOK-005-canonical-links`; le worktree est dirty et ce rapport ne prétend pas que ces changements sont dans un commit.

## Résultat observé

Le JSON-RPC MCP transmet son ID à `AgentToolPortV1::invoke_with_request_id`. Le writer de `workspace.write_text` prévalide la capacité, la base et les critères protégés, puis appelle `MissionExecutionJournalV1::execute_workspace_write`. Cette transition garde le verrou de mission pendant la validation du lease, l’append durable de l’intent, l’effet à comparaison du hash pré-état, la relecture du fichier et l’append du résultat MCP. Le résultat complet est persisté avant d’être renvoyé au client MCP. Une redelivery sous un autre ID JSON-RPC retrouve le résultat sans répéter l’écriture quand l’input et le hash observé correspondent; un hash divergent ou une opération ambiguë est refusé. Le chemin direct `invoke` pour l’écriture est refusé sans ID de corrélation.

Le test `tests::mcp_workspace_write_uses_the_journaled_fenced_writer_and_replays_exactly` passe dans le binaire `mission-012-mcp-host`. Il exerce un petit projet Rust local, l’écriture réelle du candidat, l’événement intent et résultat, le replay sous un nouvel ID sans effet ou append supplémentaire et le refus de l’entrée sans ID. Le journal conserve les reçus durablement. Le test n’utilise aucun modèle et ne touche pas Forgejo.

## Vérifications

- `cargo test --offline --locked --all-targets -- --test-threads=1` : 34 groupes, 451 passés, 0 échoué, 30 ignorés. Le test MCP ci-dessus est parmi les 23 tests du binaire MCP; le contrat journal fait 62 passés et 2 ignorés.
- `cargo test --offline --locked --test mission_execution_journal_contract` en exécution parallèle : 62 passés, 0 échoué, 2 ignorés. Le helper de contention a une échéance de 10 s, ce qui évite de confondre contention du harness avec le contrat testé.
- `cargo build --offline --locked --release --bin mission-012-mcp-host` : succès.
- `cargo clippy --offline --locked --bin mission-012-mcp-host -- -D warnings -A clippy::too_many_arguments -A clippy::collapsible_if -A clippy::bool_assert_comparison -A clippy::manual_div_ceil` : succès avec allowances historiques explicites.
- `cargo fmt --all -- --check`, `git diff --check` et `python3 scripts/verify_cargo_license_state.py` : succès. Cargo retourne `license=null`, `license_file=null`, `publish=[]`, `license_decision=unselected`, `redistribution_rights=not_assessed`.

Les sorties brutes sont dans ce répertoire : `all-targets-final.log`, `mission-journal-contract-parallel.log`, `release-build.log`, `clippy-bounded-allowances.log`, `rustfmt-check.log`, `diff-check.log` et `cargo-license-guard.json`. Les versions et hashes sont dans `toolchain.txt` et `source-sha256.txt`.

Le manifeste local de ce lot couvre 12 fichiers et passe 12/12. Après resynchronisation des nouvelles sources et des documents cités, le manifeste global compte 2 862 lignes : 2 861 hashes valides, zéro fichier manquant et zéro divergence. L’unique ligne historique malformée (`None  tests/test_mission_012_live_composed_e2e.py`) est conservée.

## Frontière de la preuve

La réconciliation de l’API/journal sait reconnaître un pré- ou post-état exact sous un nouveau fence, comme vérifié par le contrat `workspace_write_reconciles_a_post_state_under_a_newer_fence`. Cela n’établit pas un redémarrage du processus MCP. Au démarrage, `bind_workspace_to_admitted_commit` et `CandidateToolHost::new_with_mission_context` exigent encore un candidat égal à la base; les états baseline/green, compteurs et reçus de vérification sont en mémoire; le fichier de reçu est créé par `create_new`. Après SIGKILL, le candidat modifié est donc refusé avant la voie de réconciliation. Aucun hook SIGKILL, reconstruction candidate depuis le journal ou restauration de vérification n’est démontré.

Le verrou et le compare-and-replace protègent les hôtes coopératifs partageant la même identité; ils n’empêchent pas un autre processus du même UID d’ignorer le verrou ou d’agir entre les contrôles. Un hash atteste seulement l’état lu. Le test ne démontre ni le parcours LLM, ni l’intégration AgentOS/Forge Code/RAG/mémoire, ni une PR/fusion Forgejo, ni la comparaison avec CI conventionnelle. Les sources nouvelles de runtime/host/tests sont non suivies dans ce worktree; aucun commit propre n’est avancé.

## Prochaine étape

Concevoir un checkpoint de reprise versionné lié à la base admise, au hash des exigences et au journal; reconstruire le candidat depuis la base; accepter uniquement un intent non résolu et un hash source exactement pré- ou post-écriture sous un lease/fence neuf; sinon refuser. Ajouter ensuite des tests processus SIGKILL avant l’effet et après l’effet mais avant le résultat, redémarrer le binaire avec un nouveau reçu, puis refaire baseline et critères d’acceptation protégés avant de permettre la suite vers une PR.
