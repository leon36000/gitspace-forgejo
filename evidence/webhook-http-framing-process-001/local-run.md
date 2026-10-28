# WEBHOOK-HTTP-FRAMING-PROCESS-001 — cadrage HTTP ambigu refusé avant journalisation

Date : 25 octobre 2026. Worktree `local/WEBHOOK-005-canonical-links`, HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. Le test [`actual_receiver_rejects_ambiguous_message_framing_before_append`](../../tests/webhook_receiver_process_e2e.rs#L477) traverse le vrai binaire `forgejo-webhook-local`, lié uniquement sur loopback, avec une inbox temporaire privée et un corps synthétique signé par HMAC valide.

Deux requêtes ambiguës sont envoyées : deux `Content-Length` contradictoires; puis `Transfer-Encoding: chunked` avec `Content-Length`. Chaque requête reçoit HTTP 400 `request_malformed`, `retryable=false`; l’inbox reste identique octet par octet après chacun des refus, puis le replay final est vide. Le contrat de parseur `conflicting_content_length_and_chunked_transfer_are_rejected` couvrait déjà ces règles isolément; cette tranche vérifie leur effet via le processus jusqu’au stockage durable.

## Vérifications

- Test processus ciblé : 1/1 réussi, 54 tests filtrés.
- `cargo test --offline --locked --all-targets -- --test-threads=1` : code 0, 34 groupes, 501 réussis, 0 échec, 55 ignorés. Le journal complet est `all-targets-tests.log`.
- `cargo fmt --all -- --check` et `git diff --check` : code 0.
- Garde de métadonnées Cargo : 7/7 tests, `license=null`, `license_file=null`, `publish=[]`, `no_license_selected`, droits `not_assessed`.
- Source du test : SHA-256 `d02babfa2553bcec3bfb782be0b75183f9a1027696cf47f132057f0c520a371a`.

Il s’agit d’un test de récepteur local contre une fixture loopback, pas d’un test sur Forgejo vivant. Aucun changement au runtime, LLM, AgentOS, dépense, secret externe ou publication. Les 55 cas ignorés ne sont pas exécutés par `--all-targets`; leurs gardes et limites sont conservées dans les paquets de preuve correspondants.

## Suite

Poursuivre l’examen isolé du protocole LFS/fence Forgejo v17 et des limites de l’adaptateur; conserver le statut produit sans LLM réel et sans benchmark contrôlé tant qu’aucune intégration autorisée et sans coût nouveau n’est constatée.
