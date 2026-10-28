# WEBHOOK-EVENT-PAYLOAD-KIND-001

Date : 25 octobre 2026. Produit audité dans le worktree dirty `local/WEBHOOK-005-canonical-links`, basé sur HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. Cette tranche ne réattribue pas les autres modifications non commitées de ce worktree.

## Lacune et correction

La signature Forgejo/Gitea vérifie les octets du corps JSON; le nom d’événement arrive séparément dans un en-tête. Avant ce correctif, `ingest_forgejo_delivery` acceptait et persistait un corps d’issue signé sous l’en-tête `pull_request`; le rejet n’arrivait qu’au décodage ultérieur de l’inbox. Cela pouvait introduire une notification de mauvaise ressource dans la file durable.

Le nouveau test lance le vrai binaire local `forgejo-webhook-local`, signe un corps d’issue avec le secret de fixture, puis annonce `pull_request` dans l’en-tête. Avant le correctif, la commande ciblée a échoué avec le statut observé 202 au lieu de 400. L’assertion s’arrêtait alors avant la lecture de l’inbox; l’append du cas rouge est donc déduit du statut accepté et du chemin d’ingestion, pas d’un readback exécuté par ce run rouge. Après le correctif, le même test passe 1/1 et vérifie HTTP 400, le code `event_payload_invalid`, `retryable=false`, puis une inbox vide.

Le validateur partage maintenant le parseur de référence de ressource entre l’ingestion et le replay. Il exige que `issues` transporte `repository.full_name` et `issue.number`; les événements pull request exigent `repository.full_name` et `pull_request.number`. Le handler renvoie une erreur permanente et actionnable avant l’append. Un fixture existant qui appelait une livraison valide avec seulement `{"action":"opened"}` a été complété avec une issue et un dépôt; `webhook_handler_contract` passe ensuite 7/7.

## Commandes et résultats

- `cargo test --offline --locked --test webhook_receiver_process_e2e actual_receiver_rejects_a_signed_body_with_a_mismatched_event_header_before_append -- --exact --nocapture --test-threads=1` — rouge avant le correctif (202, attendu 400), puis vert après (1 réussi, 25 filtrés). Ce test démarre le vrai récepteur local, pas Forgejo.
- `cargo test --offline --locked --quiet --test webhook_handler_contract` — 7 réussis, 0 échec; sortie enregistrée dans `handler-contract-tests.log`.
- `rustfmt --edition 2024 --check src/webhook.rs src/webhook/http.rs src/webhook/reconcile.rs tests/webhook_handler_contract.rs tests/webhook_receiver_process_e2e.rs` et `git diff --check` sur ces chemins — réussis; voir `format-and-diff-checks.txt`.
- `python3 scripts/verify_cargo_license_state.py` — état Cargo vérifié dans `cargo-license-state.json`: `license=null`, `license_file=null`, `publish=[]`, décision `unselected`, droits `not_assessed`.

Les versions et empreintes des sources testées sont enregistrées dans `tool-versions.txt` et `source-sha256.txt`. `SHA256SUMS` couvre les fichiers de preuve locaux.

## Portée et limites

La vérification empêche ici une confusion de **type de ressource** issue/PR avant persistance. Elle n’authentifie pas cryptographiquement l’en-tête d’événement : les sous-types qui partagent un objet `pull_request` restent impossibles à distinguer par cette vérification seule. Le rejeu sous un nouvel ID de livraison n’est pas traité. Aucun Forgejo live, LLM, coût de modèle ou parcours complet n’est démontré; le test utilise le vrai processus récepteur avec un secret synthétique local et un journal temporaire.
