# CONTINUATION-345 — contrat d’actions du header `pull_request`

Date : 28 octobre 2026. Worktree local : `/home/pc6/Documents/Codex/2026-10-21-mission-construire-une-forge-con-ue/.runtime/webhook-005-workspace`; branche `local/WEBHOOK-005-canonical-links`; HEAD initial `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. Le checkout était déjà fortement modifié; aucun reset, commit ou changement de remote n’a été fait.

## Question et conclusion

Le runtime avait commencé à vérifier `payload.action` sous `pull_request`, mais l’allowlist locale ne contenait que `opened`, `closed`, `reopened` et `edited`. Cela refusait l’action native `synchronized` et ne couvrait pas les autres types internes que Forgejo route vers le même header. La liste courante et le document machine-readable `event_contracts_v7` couvrent maintenant treize actions. Les versions précédentes v5/v6 gardent leurs listes figées. L’action `reviewed` reste refusée ici parce que Forgejo l’émet avec des headers de revue séparés; l’action `synchronize` (singulier) est aussi refusée.

Ce résultat établit la compatibilité du validateur Rust et de l’ingestion locale avec les chaînes observées dans la source amont. Il ne démontre pas l’émission live de chacune des treize actions.

## Référence amont vérifiée

Dépôt officiel Forgejo v17.0.0, commit exact `0d791421f85ca13a63b665053d81ea765fbbc571`, worktree scratch `/tmp/forgejo-v17-base-cas-poc`. Sources lues au commit avec `git show HEAD:<path>` :

- `modules/webhook/type.go::HookEventType.Event`, lignes 68–70 : `HookEventPullRequest`, `Assign`, `Label`, `Milestone`, `Sync` et `ReviewRequest` se sérialisent tous en `pull_request`.
- `services/webhook/notifier.go::NewPullRequest`, `IssueChangeTitle`, `IssueChangeContent`, `IssueChangeStatus` : actions `opened`, `edited`, `closed`, `reopened`.
- Le même fichier, `IssueChangeAssignee`, `IssueClearLabels`, `IssueChangeLabels`, `IssueChangeMilestone`, `PullRequestReviewRequest`, `PullRequestSynchronized` : actions `assigned`, `unassigned`, `label_updated`, `label_cleared`, `milestoned`, `demilestoned`, `review_requested`, `review_request_removed`, `synchronized`.
- `modules/structs/hook.go::HookIssueAction`, lignes 292–319 : chaînes wire de l’enum partagé. Une valeur dans cet enum ne suffit pas à l’attribuer à un header; la correspondance vient des callsites et de `Event()`.

Blobs au commit : `type.go` `ecb75083f54cbdf110a76a826fb59349cfd0716a`; `notifier.go` `5aea521a0f2cb5d7513c98e9805d10e6a6b2901b`; `hook.go` `9c7f1deed1c38eeb435ab976bb8429c5458c7f2b`. SHA-256 du contenu des trois fichiers : `ca6a94e1574be77bc4f5da1ee01a3e59147a148342b958577d12544dbd6d820c`, `a67cca36f92104efe0feabf0c62e668e7543b979e5a7440e713ce93dd490004f`, `42f951d98dbea6b78e70e595b8c2cc0f5931bee2d96b837d8d4e8f034b015a12`. `git diff --exit-code HEAD -- modules/webhook/type.go services/webhook/notifier.go modules/structs/hook.go` a réussi; le scratch global demeure modifié par d’autres POC.

Portée amont : trois fichiers et les symboles/callsites listés, un seul tag; aucun test Go ni audit exhaustif du dépôt. Aucun code Forgejo n’est copié dans le produit.

## Cycle test-first et correction

Avant le changement d’allowlist, les nouvelles assertions ont reproduit le défaut :

- `cargo test --offline --locked --test webhook_handler_contract forgejo_v17_synchronized_pull_request_event_is_accepted_under_pull_request_header -- --exact --nocapture` : échec observé, HTTP 400 au lieu de 202.
- Après extension du test à la liste complète, `cargo test --offline --locked --test webhook_reconcile_contract pull_request_events_require_supported_forgejo_v17_actions -- --exact --nocapture` : échec observé sur `InvalidEventPayloadField("action")`.
- L’ingestion test-first a également rendu visible la cause d’échec du contrôle `review` : l’action inconnue arrêtait le traitement à `payload.action` avant d’atteindre `payload.review`.

La correction minimale étend `SUPPORTED_EVENT_ACTION_CONTRACTS` à `opened`, `closed`, `reopened`, `edited`, `assigned`, `unassigned`, `label_updated`, `label_cleared`, `milestoned`, `demilestoned`, `synchronized`, `review_requested`, `review_request_removed`. Le reconciler et le handler partagent cette source. Le handler exige une action textuelle autorisée avant l’append; le payload invalide garde le chemin d’erreur actionnable et n’est pas journalisé. Le contrat v7 l’annonce; v5 et v6 restent inchangés.

Le premier run complet a trouvé un fixture préexistant dans `tests/webhook_receiver_process_e2e.rs::actual_receiver_binds_pull_request_base_repository_but_allows_fork_head` : `action=synchronize` (singulier) était attendu HTTP 202. Forgejo v17 utilise `synchronized`. Les 25 fixtures Rust de ce fichier employant la forme singulière comme événement PR ont été alignés sur `synchronized`; la forme invalide est couverte explicitement dans les nouveaux cas de refus. Le log rouge initial est conservé sous `cargo-all-targets.log`; le test processus dédié passe dans le run final.

## Vérifications locales

Environnement observé : Cargo 1.95.0 (`f2d3ce0bd`, 2026-03-21), rustc 1.95.0 (`59807616e`, 2026-04-14), Linux 7.0.0, aarch64.

Commandes ciblées vertes :

- `cargo test --offline --locked --test webhook_handler_contract -- --test-threads=1` — 30/30.
- `cargo test --offline --locked --test webhook_reconcile_contract -- --test-threads=1` — 88/88.
- `cargo test --offline --locked --test forgejo_reconcile_capabilities_e2e actual_cli_discloses_versioned_local_test_capabilities -- --exact --nocapture` — 1/1.
- `cargo test --offline --locked --test webhook_receiver_process_e2e capabilities_endpoint_describes_the_versioned_webhook_contract_without_mutating_inbox -- --exact --nocapture` — 1/1.
- `cargo test --offline --locked --test webhook_receiver_process_e2e actual_receiver_binds_pull_request_base_repository_but_allows_fork_head -- --exact --nocapture` — 1/1 après alignement du fixture.

Suite finale : `cargo test --offline --locked --all-targets -- --test-threads=1` — 38 groupes, 732 réussis, 0 échec, 161 ignorés. Le log complet est [cargo-all-targets-final.log](cargo-all-targets-final.log). Le premier run avant correction de fixtures est conservé dans [cargo-all-targets.log](cargo-all-targets.log); il s’arrêtait avec le fixture `synchronize` ci-dessus. Un run complet intermédiaire, avant d’ajouter le cas de refus explicite `synchronize`, est conservé dans [cargo-all-targets-corrected.log](cargo-all-targets-corrected.log).

`cargo fmt --all -- --check` et `git diff --check` passent. `python3 scripts/test_cargo_license_state.py` passe 12/12 et `python3 scripts/verify_cargo_license_state.py` renvoie `metadata_policy_verified`. [cargo-metadata.json](cargo-metadata.json) confirme `license=null`, `license_file=null`, `publish=[]`; la politique reste `no_license_selected`, droits de redistribution `not_assessed`, marqueur SPDX `NOASSERTION`. Aucune licence n’a été choisie ni aucun manifeste Cargo modifié par ce lot.

Les SHA-256 des journaux de suite finale, suite intermédiaire, suite initiale rouge, métadonnées Cargo et tests de garde Cargo sont respectivement `77c71bf7fda1d69162d44f95bdb17e5a891e625d6fdc81f026c8163a7e8e8d53`, `989e84068f7cc28292a40ed6ef1f4fdcdc5bd0d82dd79175f125e7cf09e48aea`, `57e3f3a0e251abaafacd0edbcab0e81c7874f0db586622e99c8a44270ead0e6d`, `0afe6c6e6a8383262e8f227fb31a6286023541f6fcb3196c8dc948dc0371b0ae`, `45b1851eaa30f11140bc7f44b74e4f5c548ebe2a7a24aa9947f30bf1e2bcf153`.

## Limites et suite

Les tests d’action réutilisent des payloads JSON synthétiques. Dans ce lot, seule `synchronized` a une observation Forgejo live antérieure, conservée dans [FORGEJO-PR-SYNC-MERGE-002](../forgejo-pr-sync-merge-002/local-run.md); l’émission live des autres actions n’est pas démontrée ici. Les 161 tests ignorés n’ont pas été exécutés. Aucun AgentOS/LLM, GitLab, Tailscale ou service externe n’a été utilisé; aucun gain comparatif, nouvelle PR, vérification indépendante, fusion ou observation post-fusion n’est démontré. La prochaine tranche doit traiter une lacune locale distincte dans la matrice, sans refaire ce parcours d’action.
