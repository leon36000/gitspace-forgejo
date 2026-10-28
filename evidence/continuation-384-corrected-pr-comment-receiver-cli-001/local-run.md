# CONTINUATION-384 — retry corrigé d’un commentaire PR jusqu’au readback canonique

Date : 28 octobre 2026. Worktree `webhook-005-workspace`, branche `local/WEBHOOK-005-canonical-links`, base HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. Outils : Rust/Cargo 1.95.0, architecture aarch64.

## Scénario composé

Le test `actual_reconcile_cli_process_reconciles_same_id_corrected_pr_comment_after_receiver_refusal` lance le vrai receiver et le vrai CLI. Un `issue_comment/created` signé avec `is_pull=true` mais sans `pull_request` reçoit HTTP 400 `event_payload_invalid`; le journal reste vide. Le payload corrigé est renvoyé avec le même delivery ID et est accepté HTTP 202, `sequence=1`, `duplicate=false`; un seul événement complet apparaît dans le journal.

Le CLI lit ensuite, dans cet ordre, la PR courante et le commentaire courant via deux requêtes API distinctes, sans en-tête `Authorization`. Le webhook porte `draft=true` et des SHA anciens; les réponses API courantes portent `draft=false` et d’autres SHA. Le verdict est `current_state_read`, les champs canoniques viennent de l’API, et le corps du commentaire API garde la confiance `untrusted_user_content`. Les octets du journal ne changent pas pendant la réconciliation.

## Diagnostic et exécutions

Le premier run isolé a compilé mais échoué proprement : le CLI a renvoyé `inconclusive/invalid_current_state`, `details.field=draft`. Le contrat de réconciliation impose un booléen `draft` dans l’instantané API courant; l’omission était une fixture invalide, pas un défaut du receiver ou du CLI. La fixture a été complétée avec `draft=false`; l’événement porte volontairement `draft=true` pour vérifier la priorité de l’état courant.

- `python3 scripts/run_webhook_reconcile_cli_e2e.py --scenario actual_reconcile_cli_process_reconciles_same_id_corrected_pr_comment_after_receiver_refusal` : 1/1 sous Bubblewrap (`--unshare-net --unshare-pid`). Logs initiaux et diagnostiques en regard du run final réussi.
- `cargo test --offline --locked --test webhook_receiver_process_e2e -- --test-threads=1` : 42 réussis, 0 échec, 140 ignorés. Les tests ignorés ne sont pas exécutés par cette commande.
- `cargo fmt --all -- --check`, `git diff --check`, syntaxe du runner : réussis.
- `python3 scripts/test_cargo_license_state.py` : 12/12. `cargo metadata --offline --locked --no-deps --format-version 1` : `license=null`, `license_file=null`, `publish=[]`; aucune licence n’est sélectionnée.

## Limites

Le lot ajoute un test composé, sans changement de logique de production. HMAC et réponses API sont synthétiques; le serveur HTTP d’API est une fixture loopback et le runner retire le réseau externe. Aucun webhook n’est émis par une instance Forgejo vivante. Ce résultat ne démontre ni AgentOS/LLM, création ou fusion réelle d’une PR, ni qualité comparée ou réduction de coût. Le nombre d’ignorés limite la couverture de la cible complète.

Commandes et environnement : `verification-commands.txt`, `environment.txt`; les runs rouge/diagnostic/vert et les checks finaux sont conservés dans les journaux voisins.
