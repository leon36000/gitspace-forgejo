# CONTINUATION-082 — refus processus d’un statut workflow_job falsifié

Date : 26 octobre 2026. Produit : worktree `/home/pc6/Documents/Codex/2026-10-21-mission-construire-une-forge-con-ue/.runtime/webhook-005-workspace`, branche `local/WEBHOOK-005-canonical-links`, base `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. Worktree déjà modifié; aucun reset, commit ou changement de service externe.

## Contrôle vérifié

Forgejo signe le corps du webhook; `X-Forgejo-Event` n’est pas couvert par cette signature. Le test `actual_receiver_refuses_workflow_job_status_header_substitution_before_append` utilise le receiver Rust réel avec un corps HMAC valide contenant `job.status=failure` et un header `workflow_job_success`.

Résultats : le receiver répond HTTP 400 `event_payload_invalid`, `retryable=false`, et le journal reste vide. Il reçoit ensuite le même ID de livraison et les mêmes octets signés avec le suffixe correct `workflow_job_failure`; cette fois HTTP 202, `duplicate=false`, séquence 1, et exactement un événement `forgejo.webhook.v1.workflow_job_failure` est journalisé. Le refus du header n’a donc ni accepté le statut falsifié ni réservé l’idempotency key.

Le scénario dédié a été exécuté via `scripts/run_webhook_reconcile_cli_e2e.py` sous Bubblewrap 0.9.0 avec `--unshare-net`, `--unshare-pid`, environnement effacé et source montée en lecture seule; `process-e2e.log` donne 1 réussi, 97 filtrés. Une première invocation du driver avait terminé avec code 0 mais exécuté 0 test parce que le nouveau cas n’avait pas encore l’attribut `#[ignore]` attendu par son filtre. Ce reçu est conservé comme `runner-filter-misfire.log`; après correction, le test exact a été lancé et réussi. Un code de sortie zéro du driver seul ne suffit donc pas à établir qu’un scénario a été exécuté.

## Vérifications cumulatives

- `cargo test --offline --locked --all-targets -- --test-threads=1 --quiet` : 596 réussis, 0 échec, 90 ignorés, 34 exécutables (`all-targets.log` et `test-summary.txt`). Le scénario processus ci-dessus est exécuté séparément puisqu’il est marqué `ignore` pour exiger le sandbox.
- `cargo fmt --all -- --check`, `git diff --check` et py_compile du driver passent.
- `python3 scripts/verify_cargo_license_state.py` passe; `python3 -m unittest -v scripts.test_cargo_license_state` passe 9/9. La licence Cargo reste non choisie (`license=null`, `license_file=null`, `publish=[]`, `no_license_selected`); les droits de redistribution restent `not_assessed`, export SPDX `NOASSERTION`.
- Les manifestes locaux et append-only produit/parent sont validés dans `manifest-check.txt` et `global-manifest-check.txt`; les préfixes sont conservés.

## Portée et limites

Le receiver est un vrai processus local; le test crée un webhook synthétique signé. Aucun événement n’est émis par Forgejo, aucun runner Actions, exécuteur LLM/AgentOS, RAG ou mémoire n’est appelé. Le `PATH` testé ne contient pas `forgejo`, `act_runner` ni `gitea`, donc ce lot ne prétend pas valider l’émission native. Aucun coût, secret externe, publication ou production n’est impliqué. Cette preuve couvre la correspondance entre le header et `job.status` dans ce chemin processus; elle ne garantit pas la validité générale d’une livraison Forgejo.
