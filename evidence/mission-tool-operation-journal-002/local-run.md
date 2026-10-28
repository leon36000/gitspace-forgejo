# MISSION-012-TOOL-OPERATION-JOURNAL-002

Date : 25 octobre 2026. Worktree courant sur HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`, branche `local/WEBHOOK-005-canonical-links`; les fichiers agent-runtime/host/test sont non suivis et le dépôt est fortement modifié. Aucun commit ni publication n’a été créé.

## Défaut visé et correction

L’intent d’une écriture est journalisé avant l’effet, puis le résultat après readback. Lors d’une interruption dans cette fenêtre, le code pouvait détecter le replay exact de l’input, mais ne bloquait pas une nouvelle écriture différente sous un bail récent. Cela permettait plusieurs intents sans résultat dans la même génération, rendant la reprise ambigüe.

Le test rouge `unresolved_workspace_write_must_be_replayed_before_a_different_write` a d’abord échoué : `begin_workspace_write_intent` a retourné un second `WorkspaceWriteIntentV1` au lieu de refuser. La fonction commune `validate_unresolved_workspace_write` examine désormais les intents non résolus pour la mission/génération. Elle refuse l’ambiguïté de plusieurs intents, exige la sélection de l’unique opération exacte, et signale `tool_operation_recovery_pending` pour un input différent avec l’ID, le hash observé et les hashes pré/post. Si l’input est identique mais que le hash observé n’est ni pré ni post, le refus existant `tool_operation_effect_diverged` est conservé. Les conflits de corrélation JSON-RPC sont vérifiés avant le refus générique et gardent `tool_operation_request_id_conflict`.

La garde est appelée par les deux chemins, `begin_workspace_write_intent` et `execute_workspace_write`. Le test sous un nouveau fence exerce les deux API, vérifie un message non réessayable avec action explicite, constate que le contenu du workspace ne change pas et qu’un seul intent persiste. Les tests existants couvrent toujours le replay exact de l’input au pré-état et la promotion du post-état sous fence plus récent.

## Vérifications

- Test rouge avant code : `cargo test --offline --locked --test mission_execution_journal_contract unresolved_workspace_write_must_be_replayed_before_a_different_write -- --nocapture` — échec attendu car le nouvel intent était accepté.
- Test ciblé final de la garde — 1 réussi.
- Contrat journal : `cargo test --offline --locked --test mission_execution_journal_contract -- --test-threads=1` — 63 réussis, 0 échec, 2 ignorés.
- Suite : `cargo test --offline --locked --all-targets -- --test-threads=1` — 454 réussis, 0 échec, 30 ignorés, 34 groupes.
- Build : `cargo build --offline --locked --release --bin mission-012-mcp-host` — réussi.
- Clippy strict all-targets — échec sur deux lints de tests hors de cette tranche : `enum_variant_names` dans `tests/forgejo_merge_post_crash_recovery_process_e2e.rs` et `type_complexity` dans `tests/webhook_receiver_process_e2e.rs`. Rejoué avec six allowances explicites (`too_many_arguments`, `collapsible_if`, `bool_assert_comparison`, `manual_div_ceil`, `enum_variant_names`, `type_complexity`) — réussi. Les deux sorties sont archivées.
- `cargo fmt --all -- --check` et `git diff --check` — réussis.
- `cargo metadata --offline --no-deps --format-version 1` — `license=null`, `license_file=null`, `publish=[]`; metadata `agent-forge.licensing` reste `decision=unselected`, `redistribution_rights=not_assessed`.

## Versions et limites

Outils : Rust/Cargo 1.95.0, Python 3.12.3, Git 2.43.0; Linux 7.0.0-1019-nvidia aarch64. Voir `toolchain.txt` et les empreintes de `source-sha256.txt`.

Ce lot teste la logique de journal avec un contrat Rust local; il ne redémarre pas le processus hôte et ne restaure pas un workspace candidat. `src/bin/mission-012-mcp-host.rs` appelle encore `bind_workspace_to_admitted_commit` avant l’acquisition du lease, et `CandidateToolHost` garde baseline/acceptation en mémoire. Il n’y a pas de test SIGKILL, de PR ou de merge Forgejo dans ce lot. La recherche dans `src/` et `Cargo.toml` ne trouve pas de connecteur modèle/AgentOS/Forge Code/RAG/mémoire; aucun LLM ou service externe n’a été utilisé.

Prochaine étape : checkpoint de reprise lié à l’admission, exigences, base et journal; reconstruire le candidat depuis la base seulement après validation d’un unique intent et d’un nouveau fence; injecter SIGKILL avant effet et après effet avant résultat, refuser un hash tiers, rejouer l’input exact, puis refaire baseline et acceptation avant PR. Garder les changements locaux existants; ne pas rejouer les parcours 046–050.
