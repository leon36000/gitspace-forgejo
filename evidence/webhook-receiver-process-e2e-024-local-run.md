# WEBHOOK-024 — conflit de redelivery au travers du processus TCP

Date : 2026-10-22. Worktree : `local/WEBHOOK-005-canonical-links`. HEAD examiné : `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`.

## Comportement observé

Le contrat existant `actual_receiver_process_accepts_retries_recovers_and_reconciles` de `tests/webhook_receiver_process_e2e.rs` couvre maintenant un conflit au niveau du vrai binaire `forgejo-webhook-local` : après une livraison valide et son retry identique, il envoie le même delivery ID avec un corps différent et un HMAC valide. Le processus répond `409 delivery_conflict` avec `retryable=false`. À l’arrêt, le journal vérifié contient toujours une seule ligne, celle du premier corps. La réconciliation de l’événement conservé utilise l’état courant `open` d’une fixture, malgré le payload `closed`; après redémarrage, le retry original reste reconnu comme doublon et le journal contient encore une ligne.

Le changement de code est uniquement un renforcement du test d’intégration; aucun code de production n’a changé. Le secret, l’issue, le journal et le serveur d’état sont synthétiques et locaux. La signature atteste seulement que le test a envoyé un corps cohérent avec sa clé fixture.

## Vérifications

- `CARGO_TARGET_DIR=/tmp/webhook-receiver-process-e2e-target cargo test --offline --locked --test webhook_receiver_process_e2e -- --nocapture` : 1/1 réussi; [journal](webhook-receiver-process-e2e-024-focused.log).
- `CARGO_TARGET_DIR=/tmp/webhook-receiver-process-e2e-target cargo test --offline --locked --all-targets` : 23 suites, 209 réussis, 0 échec, 2 helpers subprocess ignorés; [journal](webhook-receiver-process-e2e-024-all-targets.log).
- `cargo fmt --all -- --check` : réussi; [journal](webhook-receiver-process-e2e-024-rustfmt.log).
- `git diff --check` : réussi; [journal](webhook-receiver-process-e2e-024-diff-check.log).
- `CARGO_TARGET_DIR=/tmp/webhook-receiver-process-e2e-target cargo clippy --offline --locked --all-targets -- -D warnings -A clippy::too_many_arguments -A clippy::bool_assert_comparison -A clippy::manual_div_ceil` : réussi avec ces trois catégories préexistantes explicitement neutralisées; [journal](webhook-receiver-process-e2e-024-clippy-with-baseline-allowances.log). Cela ne prouve pas que Clippy strict sans exception passe.

## Limites

Le serveur courant de réconciliation reste une fixture : ce contrat ne contacte pas l’instance Forgejo, n’exerce pas de credential réel, de hook initié par Forgejo, d’adaptateur HTTP de réconciliation dans un deuxième processus, de PR, de vérification indépendante, de fusion ou de modèle. Il vérifie le refus du conflit au serveur récepteur, la persistance et la reprise du reçu original dans le journal local. Ce résultat n’établit pas un gain général de qualité, coût ou délai.
