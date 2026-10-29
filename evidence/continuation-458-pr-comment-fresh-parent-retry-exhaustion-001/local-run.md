# CONTINUATION-458 — refus après épuisement des retries du parent frais

Date : 29 octobre 2026. Worktree : `/home/pc6/Documents/Codex/2026-10-21-mission-construire-une-forge-con-ue/.runtime/webhook-005-workspace`; branche `local/WEBHOOK-005-canonical-links`; HEAD de base `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. Le checkout était déjà fortement modifié; aucune modification préexistante n’a été annulée, écrasée ou commise.

## Scénario et résultat

Le test `actual_reconcile_cli_process_keeps_deleted_pr_comment_inconclusive_after_fresh_parent_retries_exhaust` compose le receiver Rust, son journal durable, le vrai binaire `forgejo-reconcile-local` et une fixture HTTP loopback. Un événement signé `issue_comment/deleted` lié à une PR reçoit un parent initial valide, puis un 404 sur la lecture du commentaire. Les trois tentatives de relecture fraîche du parent répondent 503 avec `Retry-After: 0`.

Sous Bubblewrap avec namespaces réseau et PID privés, le scénario observe exactement cinq GET : parent initial, commentaire, puis trois GET parent. Le résultat est `inconclusive/upstream_http_status`, `retryable=true`, `http_status=503`, `upstream_read_attempts=5`, sans `canonical_state`. Aucune Authorization n’est émise, aucun corps HTTP sentinelle ou texte du webhook ne fuite, et l’inbox reste byte-for-byte identique. Le processus isolé passe **1/1**. Le test est `#[ignore]` dans Cargo et s’exécute séparément via l’allowlist `scripts/run_webhook_reconcile_cli_e2e.py`.

La suite verrouillée/hors ligne `cargo test --locked --offline --all-targets -- --test-threads=1` passe **39 groupes, 793 réussis, 0 échec, 218 ignorés**. Les tests processus ignorés ne sont pas présentés comme exécutés dans ce décompte; celui de CONTINUATION-458 passe séparément. `cargo fmt --all -- --check`, `git diff --check`, la syntaxe du runner et du helper de manifeste, les liens documentaires ciblés et le garde Cargo passent. Journaux, versions, commandes, résumé machine, hashes et reçus sont dans ce dossier.

## Portée et limites

Ce lot a ajouté un test processus et son allowlist, actualisé les documents canoniques et enregistré les preuves; aucun code runtime n’a changé. Les réponses sont synthétiques et loopback (`n=1`), pas émises par Forgejo live. Le scénario démontre un refus après panne, pas la récupération automatique ou le rejeu réussi ultérieur du même delivery. Il ne démontre ni agent LLM/AgentOS, ni fusion, ni gain comparatif. Cargo demeure sans licence choisie (`license=null`, `license_file=null`, `publish=[]`, droits `not_assessed`, SPDX `NOASSERTION`).

Prochaine tranche locale : relancer le même delivery déjà durable après rétablissement du serveur, sans nouvelle insertion dans l’inbox, et vérifier qu’un parent frais valide autorise alors `comment.state=absent`.
