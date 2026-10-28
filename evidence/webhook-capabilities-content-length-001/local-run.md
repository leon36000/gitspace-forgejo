# WEBHOOK-CAPABILITIES-CONTENT-LENGTH-001 — requête GET sans corps

Date : 26 octobre 2026. Travail dans le worktree local déjà fortement modifié, HEAD de base `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. Aucun serveur ou dépôt de production n’a été touché.

## Défaut observé

`parse_http_request` exigeait exactement un `Content-Length` pour toute requête HTTP. Le GET de découverte `GET /forgejo/capabilities` échouait donc avec `HttpRequestError::Malformed` si un client HTTP omettait cet en-tête pour un corps vide.

Le test d’intégration handler existant a été adapté en premier pour envoyer le GET ordinaire sans `Content-Length`; le nouveau test du parseur vérifie le corps vide ainsi que le refus d’octets non déclarés. Les deux nouvelles assertions ont échoué avant la correction :

- `cargo test --offline --locked --test webhook_http_contract absent_content_length_means_empty_body_and_rejects_undeclared_bytes -- --exact --nocapture` : code 101, `unwrap()` a observé `Err(Malformed)`.
- `cargo test --offline --locked --test webhook_handler_contract capabilities_publish_machine_readable_contracts_for_every_supported_event -- --exact --nocapture` : code 101, même refus au parseur avant l’accès au handler.
- Après correction, le test du processus réel `forgejo-webhook-local` `capabilities_endpoint_describes_the_versioned_webhook_contract_without_mutating_inbox` passe 1/1 avec le GET sans longueur.

La correction interprète l’absence de `Content-Length` comme un corps vide. Elle refuse tout octet après l’en-tête dans ce cas; les longueurs dupliquées, les valeurs mal formées et `Transfer-Encoding` restent refusés. Les limites de taille et le cadrage explicite des corps annoncés restent inchangés.

## Vérifications

- Test nouveau du parseur : 1/1.
- Test du handler de capacités avec un GET sans longueur : 1/1.
- `webhook_http_contract` : 7/7.
- `webhook_handler_contract` : 22/22.
- `cargo test --offline --locked --all-targets` : 577 réussis, 0 échec, 85 ignorés, 34 groupes. Le test process E2E ci-dessus est aussi exercé ici.
- `cargo fmt --all -- --check` et `git diff --check` : réussis. Le premier check Rustfmt avait signalé seulement le format d’un appel dans le test; la ligne a été formatée puis le check relancé avec succès.
- `python3 scripts/verify_cargo_license_state.py` : `verified`; `python3 -m unittest -v scripts.test_cargo_license_state` : 9/9.
- `cargo metadata --locked --offline --format-version 1 --no-deps` : `license=null`, `license_file=null`, `publish=[]`. La métadonnée locale indique `no_license_selected`, droits `not_assessed`, marqueur SPDX `NOASSERTION`. Aucune licence n’a été choisie.

Versions, hashes des sources modifiées, logs des vérifications vertes, métadonnées Cargo et SHA256SUMS sont conservés dans ce dossier.

## Portée et limites

Le correctif concerne le parseur HTTP du receiver local et rend la route de découverte utilisable par des clients HTTP qui n’envoient pas de longueur pour GET sans corps. Le comportement a été vérifié avec le vrai parseur et le handler; cette tranche ne redémarre pas une instance Forgejo, ne vérifie pas tous les clients HTTP et ne change pas le contrat d’authentification. L’audit d’interface local retrouve `AgentRuntimeV1` dans `src/agent_runtime.rs`, mais la seule implémentation trouvée est le `FixtureRuntime` de test. `ollama`, `llama-server`, `lmstudio`, `opencode`, `aider` et `goose` sont absents du PATH; l’exécutable `claude` est présent, mais n’a pas été appelé vu l’absence de plafond de dépense fiable. Aucun runtime modèle ou AgentOS n’est donc intégré dans ce lot. L’API Forgejo live, le parcours complet où le processus MCP édite le candidat fusionné et le benchmark conventionnel ne sont pas exercés ici. Le dépôt reste incomplet.
