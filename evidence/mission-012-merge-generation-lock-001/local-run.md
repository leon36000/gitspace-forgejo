# MISSION-012 — verrou de génération jusqu’à la décision de fusion

Date : 24 octobre 2026. Source examiné : HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`, branche locale `local/WEBHOOK-005-canonical-links`; le worktree comporte d’autres modifications non commitées qui ont été préservées. La forge testée est Forgejo `17.0.0+gitea-1.22.0`, binaire SHA-256 `e5ec437cd6357e8d1b4580cb47ab1daa58a32b33971a63db63446a535d33adb7`.

## Correction

La revue de l’autorité a relevé une course entre `verify_claimed_request` et le POST de fusion : un nouvel allocateur coopératif pouvait supplanter le claim après la vérification. `MissionExecutionJournalV1::with_current_claimed_request` acquiert désormais le verrou per-mission de l’allocateur, relit et valide le claim sous verrou, puis garde ce verrou pendant un callback hôte borné. Pour MISSION-012, le callback contient la relecture de l’admission par hash, les vérifications finales PR/base/policy, le POST de merge et les readbacks. Le callback ne doit pas réentrer dans le journal pour cette mission et doit avoir une échéance.

`tests/mission_execution_journal_contract.rs` vérifie qu’un allocateur concurrent reçoit `execution_mission_busy` pendant le callback puis alloue la génération suivante après libération, et qu’un claim déjà supplanté refuse sans appeler le callback. Le test de refus stale est nouveau.

## Résultats reproductibles

- `cargo test --offline --locked --all-targets --quiet` : 32 groupes, 382 réussis, 0 échec, 14 ignorés; la sortie brute est `all-targets.log`.
- `PYTHONDONTWRITEBYTECODE=1 python3 -m unittest tests.test_mission_012_live_agent_pr_e2e tests.test_mission_012_live_composed_e2e -q` : 43 tests réussis; sortie dans `python-targeted.log`.
- `cargo fmt --all -- --check`, compilation Python ciblée et `git diff --check` : code 0; sorties conservées séparément.
- Les manifestes SHA-256 des runs 028/029 et admissions correspondantes ont été contrôlés. Le run [028](../mission-012-live-agent-webhook-local-adapter-028/local-run.md) avance `main` via une PR concurrente approuvée, puis le gate Rust refuse le candidat (`invalid_review_base_binding`, autorisation et tentative à `false`, PR cible/issue ouvertes, source cassée inchangée). Le run [029](../mission-012-live-agent-webhook-local-adapter-029/local-run.md) exerce le chemin positif courant jusqu’au merge et au readback postmerge.

## Limites

Ces deux parcours utilisent `local-test-adapter`, sans LLM, AgentOS, Forge Code, RAG ou mémoire; ils ne mesurent aucune amélioration de valeur. La serrure n’exclut que les allocateurs coopératifs du même hôte. Un processus qui ignore `flock` ou un writer Forgejo externe peut contourner cette coordination; Forgejo v17 n’offre pas à ce POST le CAS atomique requis pour fermer la course après la dernière lecture. Le run négatif couvre une base avancée avant la décision, pas cette dernière fenêtre contre un writer externe. Les processus partagent l’UID local.
