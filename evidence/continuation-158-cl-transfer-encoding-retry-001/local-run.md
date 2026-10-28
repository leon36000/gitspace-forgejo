# CONTINUATION-158 — reprise après refus `Content-Length` + `Transfer-Encoding`

Date : 26 octobre 2026. Worktree `.runtime/webhook-005-workspace`, HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. Le worktree contenait déjà des modifications; ce lot n’en a remplacé ni supprimé aucune.

## Scénario et résultat

Le test `tests/webhook_receiver_process_e2e.rs::actual_receiver_rejects_content_length_with_transfer_encoding_before_corrected_retry` démarre le vrai binaire local `forgejo-webhook-local` sur loopback. Il prend une requête `issues` synthétique et HMAC-valide, dont le cadrage normal comporte un `Content-Length`, puis ajoute `Transfer-Encoding: chunked`.

Le receiver répond HTTP 400 `request_malformed`, avec `retryable=false` et l’action qui demande de retirer `Transfer-Encoding`. Les octets de l’inbox restent identiques. Une requête correctement cadrée, portant le même ID et le même payload signé, est alors acceptée HTTP 202, `duplicate=false`, à la séquence 1. Le replay final contient un seul événement `forgejo.webhook.v1.issues` avec ce delivery ID.

Commande : `cargo test --offline --locked --test webhook_receiver_process_e2e actual_receiver_rejects_content_length_with_transfer_encoding_before_corrected_retry -- --exact`. Résultat enregistré : **1 réussi, 0 échec, 123 filtrés**, test 0,07 s.

Ce lot ajoute un contrôle de processus sur la non-réservation d’ID après un refus de framing CL+TE. Le parseur existant réalise déjà le refus; aucun changement de runtime n’a été fait. Le contrôle complète le cas existant de longueurs contradictoires et lie le message d’action de CONTINUATION-157 à cette autre forme de framing ambigu.

## Vérifications

- Test ciblé : réussi; sortie dans `process-test.log`.
- `rustfmt --edition 2024 --check tests/webhook_receiver_process_e2e.rs` : réussi.
- `git diff --check` : réussi.
- `python3 scripts/verify_cargo_license_state.py` : réussi; `license=null`, `no_license_selected`, `NOASSERTION`, droits de redistribution `not_assessed`.
- Les scripts d’indexation vérifient le dossier local et préservent les préfixes précédents des manifestes worktree et projet.

Environnement : rustc 1.95.0, Cargo 1.95.0, Python 3.12.3, Linux aarch64. HEAD et hashes des sources/documents : `toolchain-and-head.txt` et `source-sha256.txt`.

## Limites

Un scénario de receiver local avec payload et secret synthétiques (`n=1`). Il ne place pas de reverse proxy devant le receiver, ne compare pas plusieurs parseurs HTTP et ne prouve pas l’absence de request smuggling dans un déploiement réel. Pas de Forgejo live, LLM/AgentOS, benchmark ni modification de production. La mission reste ouverte.
