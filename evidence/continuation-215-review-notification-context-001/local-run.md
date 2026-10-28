# CONTINUATION-215 — notification de revue conservée comme contexte non fiable

Date : 27 octobre 2026. Cible : worktree local `.runtime/webhook-005-workspace`, HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. Le checkout est fortement modifié (1 572 entrées `git status --short` au relevé); le diff agrégé ne peut pas être attribué à ce lot. Aucun commit, reset, publication, accès externe ou changement de production n’a été fait.

## Lacune et changement

Le webhook de revue v17 peut transporter `review.type` et `review.content`, mais le GET canonique de PR ne contient pas le texte de l’avis. La couche de réconciliation le projette désormais séparément dans `review_notification`; elle continue de construire `canonical_state` depuis le GET PR. Le texte est marqué `untrusted_user_content`, sa fraîcheur `event_snapshot_not_canonical`; le SHA de tête du payload est conservé uniquement comme `pull_request_head_sha_at_event`. Le webhook Forgejo v17 ne fournit pas d’ID de revue : `review_id_binding=unavailable_in_forgejo_v17_webhook`. Le texte ne vaut donc ni approbation, ni preuve du commit réellement évalué, ni autorité de fusion.

Les contenus `review.content` chaîne/null sont acceptés; une valeur non textuelle est refusée avant le GET. Les sorties API ont changé de schéma : 9 unitaire et 10 batch. Les validateurs Python et les assertions de contrat/processus ont été migrés. Aucun nouveau crate ou paquet n’a été ajouté pour ce lot.

## Vérifications effectuées

- `cargo test --locked --offline --all-targets -- --test-threads=1` : 668 réussis, 0 échec, 124 ignorés, 36 groupes. Sortie complète : `cargo-all-targets.log`.
- `cargo test --locked --offline --test webhook_reconcile_contract` : 75/75 (également inclus dans la suite complète).
- `tests/webhook_quarantine_cli.rs` : 5/5 (inclus dans la suite complète, après mise à jour des attentes 9/10).
- Processus ciblé : `python3 scripts/run_webhook_reconcile_cli_e2e.py --scenario actual_reconcile_cli_process_routes_pull_request_review_comment_to_pull_request_api` : 1/1. Le runner compile les binaires de travail dans Bubblewrap puis exécute le vrai processus CLI avec `--unshare-net --unshare-pid`; seule une fixture HTTP loopback est accessible. La fixture injecte une note de revue avec du texte hostile, vérifie que le GET retourne l’état PR courant indépendamment, et vérifie la projection distincte et non fiable. Ce n’est pas une instance Forgejo live.
- Validateurs de sortie liés : `python3 -m unittest discover -s tests -p 'test_forgejo_emitted_webhook_e2e.py'` : 131/131; `python3 -m unittest discover -s tests -p 'test_forgejo_live_org_team_reviewer_readback.py'` : 29/29; `python3 -m unittest discover -s tests -p 'test_forgejo_webhook_native_replay_e2e.py'` : 30/30. `py_compile` sur les validateurs et tests migrés passe.
- `cargo fmt --all -- --check`, `git diff --check`, `python3 scripts/verify_cargo_license_state.py` et ses 9 tests passent.
- Les manifestes append-only locaux, worktree et projet ont été prolongés sans réécriture de préfixe; le vérificateur confirme 9 fichiers de preuve, les trois préfixes inchangés et le lien parent vers l’index du worktree.
- Cargo metadata (`--locked --offline --no-deps`) : package `agent-forge-core 0.1.0`, `license=null`, `license_file=null`, `publish=[]`; revue locale `no_license_selected`, droits `not_assessed`, marqueur SPDX `NOASSERTION`.

## Corrections de harnais observées avant le run final

Le premier `cargo test --all-targets` a trouvé une attente single/batch ancienne dans le test de quarantaine (8/9). Les trois suites Python ont ensuite révélé plusieurs validateurs codés en dur à 8/9; ils attendaient maintenant 9/10. Après migration, elles passent aux effectifs ci-dessus. Un passage Rust ultérieur a également refusé une fixture `issue_comment` qui portait incorrectement `action=reviewed`; la fixture a été corrigée en `action=created`, et le test processus ciblé a passé 1/1. Le run complet final (668/0/124) est exécuté après ces corrections.

Ces échecs concernaient des contrats/fixtures de test incohérents avec les versions ou événements courants. Aucun de leurs résultats n’est présenté comme une défaillance observée de Forgejo live.

## Limites et suite

Le processus vérifié exerce un seul scénario de revue avec API loopback fixture; le contrat Rust couvre le binding de sous-type, la séparation canonical/event et le refus du type mal formé. Aucun Forgejo live, runner Actions, agent LLM/AgentOS, PR réelle, approbation indépendante, gate, fusion, mesure de tokens/coût ou benchmark comparatif n’est démontré ici. Les 124 tests ignorés demandent leurs runners dédiés. La migration de schéma peut exiger l’adaptation de clients externes non présents dans ce dépôt.

Prochaine action reprise du checkpoint : diagnostiquer le prérequis local `systemd-run/systemctl` bloquant le gate same-host courant, puis traiter les refus et le benchmark encore ouverts. Cette preuve de contexte ne ferme aucun de ces critères; le projet reste incomplet.
