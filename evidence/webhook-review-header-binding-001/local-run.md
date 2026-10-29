# WEBHOOK-REVIEW-HEADER-BINDING-001 — refus d’une substitution d’en-tête d’avis

Date : 28 octobre 2026. Worktree : branche `local/WEBHOOK-005-canonical-links`, HEAD de départ `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`; l’arbre était déjà fortement modifié. Aucun historique, branche, service ou code runtime n’a été changé dans ce lot.

## Question et constat dans le code

Forgejo calcule le HMAC du corps. Les en-têtes `X-Forgejo-Event` et `X-Gitea-Event` restent hors de cette signature. Dans `src/webhook/reconcile.rs::validate_event_payload_kind`, le runtime relie les trois en-têtes de review à `review.type` (`pull_request_approved` ↔ `pull_request_review_approved`, etc.) et rejette une discordance comme enveloppe invalide avant l’ajout durable. Le contrat avait des cas positifs et des refus d’action manquante, mais le test `unsigned_event_header_substitution_is_refused_when_payload_shape_disagrees` ne couvrait que d’autres formes d’événements. Le défaut était une lacune de preuve de refus, pas un comportement fautif constaté.

## Changement et résultat

Ajout de `unsigned_review_header_substitution_is_refused_and_same_delivery_can_retry` à `tests/webhook_handler_contract.rs`. Pour les six combinaisons hors diagonale entre body signed `approved`, `rejected` et `comment`, le test signe le JSON, remplace seulement les deux alias d’en-tête, puis appelle le vrai `handle_webhook_request` Rust. Chaque tentative est HTTP 400 `event_payload_invalid`, `retryable=false`, et ne fait pas croître le journal. Le corps inchangé est ensuite envoyé avec l’en-tête correspondant sous le même delivery ID : il est accepté et obtient les séquences 1 à 6. La matrice de refus et `docs/checkpoint.md` consignent le cas et ses limites.

Aucun code de validation runtime n’a été modifié : l’inspection et le test confirment la règle déjà présente.

## Vérifications

- Test ciblé : 1/1 passé.
- `cargo test --offline --locked --test webhook_handler_contract -- --test-threads=1` : 31 réussis, 0 échec, 0 ignoré.
- `cargo fmt --all -- --check` : un premier contrôle a relevé uniquement le formatage d’une assertion nouvelle; après correction manuelle, le contrôle final passe. Les deux sorties sont conservées.
- `git diff --check` : passe.
- Audit des liens locaux dans les cinq documents canoniques : 1 609 liens, zéro cible manquante.
- `cargo metadata --format-version 1 --locked --offline --no-deps` : code 0.
- `python3 scripts/verify_cargo_license_state.py` : passe, `license=null`, `license_file=null`, `publish=[]`, `no_license_selected`, droits `not_assessed`, SPDX `NOASSERTION`.
- Le premier build du test a signalé une assertion qui lisait `sequence` sur le wrapper `JournaledWebhookEventV1`; elle lit maintenant `event().sequence`. Le test ciblé puis la suite contrat complète passent.
- Le contrôle de manifeste du lot précédent a d’abord été invoqué depuis la racine avec des chemins relatifs à son dossier de preuve; il a échoué à ouvrir ces chemins, sans toucher aux fichiers. Relancé depuis le dossier de preuve, les 20 entrées vérifient `OK`.

Les commandes, versions, logs et hashes de sources sont joints. Le SHA-256 local du dossier et ses ajouts append-only sont vérifiés dans `SHA256SUMS` et `manifest-check.txt`.

## Limites

C’est un test d’intégration de la fonction récepteur Rust avec une requête HTTP sérialisée en mémoire et un secret HMAC synthétique. Il ne lance pas le binaire receiver sous sandbox, n’interpose pas un proxy réseau et ne teste pas un webhook émis par Forgejo. Les six paires testées couvrent seulement les trois sous-types d’avis. Le cas n’établit ni parcours LLM/AgentOS, ni comportement de fusion, ni gain comparatif.

Le connecteur GitLab PC4 fonctionne en lecture pour le projet privé et sa branche. Aucun secret du courriel n’a été lu. La pipeline 328 au commit `9ac877767e9b458a9cd911e67a10f87aeeaefbdb` reste en échec Gitleaks. Dix sorties contiennent des chaînes de 64 caractères ressemblant à des tokens Forgejo de test; leur validité/révocation et leur classification n’ont pas été établies. Aucun nouveau commit distant ni nettoyage n’a été tenté en attendant la décision de l’utilisateur.
