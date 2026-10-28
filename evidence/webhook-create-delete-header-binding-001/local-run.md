# WEBHOOK-CREATE-DELETE-HEADER-BINDING-001 — croisement du type d’événement

Date : 26 octobre 2026. Worktree local isolé `.runtime/webhook-005-workspace`, HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. Le worktree contient beaucoup de changements antérieurs; ce lot n’en remplace aucun. Pas de commit, publication ou déploiement.

## Constat source

Le checkout scratch Forgejo a pour HEAD le commit officiel v17.0.0 `0d791421f85ca13a63b665053d81ea765fbbc571`, mais son arbre de travail est fortement modifié par les POC LFS/CAS. Les fichiers utilisés ci-dessous ont été lus avec `git show HEAD:<chemin>` : les lignes examinées correspondent donc aux blobs du commit officiel, pas aux changements locaux du scratch.

- `modules/structs/hook.go:131-138` définit `CreatePayload` avec `Sha`; `:160-167` définit `DeletePayload` sans `Sha`. SHA-256 du contenu amont : voir `source-sha256.txt`.
- `services/webhook/shared/payloader.go::AddDefaultHeaders`, `:140-165`, calcule les HMAC sur `payloadContent`, puis ajoute `X-Forgejo-Event` et `X-Gitea-Event` dans des en-têtes séparés. L’en-tête choisi n’est donc pas couvert par ce HMAC.
- Le Rust recevait le type par cet en-tête et validait la forme de `delete` sans interdire `sha`. Un corps `create` signé pouvait ainsi être relu comme un événement `delete` si l’en-tête non signé était substitué.

## Changement et résultat observable

`validate_event_payload_kind` refuse désormais un événement `delete` contenant le champ create-only `sha`, avant toute lecture API. Le même contrôle s’applique à une livraison fraîche et à la relecture d’un événement. Le contrat publié par `/forgejo/capabilities` indique cette contrainte. Aucun comportement Forgejo n’est modifié.

Les tests ajoutés ont reproduit deux défauts avant la correction :

- `unsigned_delete_header_cannot_reclassify_a_signed_create_payload` recevait HTTP 202 au lieu de 400. Le test rouge s’est arrêté sur le statut avant son assertion d’absence d’écriture au journal; `ingest-red.log` conserve la sortie.
- `replayed_delete_event_rejects_create_only_sha_before_api_calls` recevait `Ok(ReconciledState)` au lieu de `UnexpectedEventPayloadField("sha")`; `replay-red.log` conserve la sortie.

Après correction, les suites complètes concernées passent :

- `cargo test --locked --offline --test webhook_handler_contract -- --test-threads=1` — 21 réussis, 0 échec.
- `cargo test --locked --offline --test webhook_reconcile_contract -- --test-threads=1` — 39 réussis, 0 échec.
- `cargo test --locked --offline --test webhook_reconcile_pipeline -- --test-threads=1` — 3 réussis, dont un nouveau parcours signé `create` → journal → lecture de branche actuelle.
- `cargo test --locked --offline --test webhook_receiver_process_e2e actual_receiver_rejects_create_payload_relabelled_as_delete_before_append -- --exact --test-threads=1` — 1/1. Le vrai binaire `forgejo-webhook-local` reçoit le POST sur loopback, retourne 400, signale `payload.sha`, puis son inbox relu est vide.
- `cargo fmt --all -- --check` et `git diff --check` — réussis.
- `cargo metadata --locked --offline --no-deps --format-version 1` — `license=null`, `license_file=null`, `publish=[]`; `no_license_selected`, `redistribution_rights=not_assessed`.
- Le garde `python3 scripts/verify_cargo_license_state.py` passe (`status=verified`); `python3 -m unittest -v scripts/test_cargo_license_state.py` passe 9/9, y compris le refus d’un identifiant de licence ou de droits évalués. Aucune licence n’est sélectionnée.

## Portée et limites

Les tests sont des contrats Rust locaux avec un secret et des événements synthétiques; aucun serveur Forgejo n’a émis le webhook pendant ce lot. Le test composé valide un `create` signé avec un adaptateur d’état canonique fixture et prouve que le SHA courant prime sur le SHA du payload; ce n’est pas une intégration à Forgejo live. Les tests démontrent les refus aux frontières d’ingestion et de réconciliation du produit, pas une propriété cryptographique générale ni la correction de tous les couples événement/en-tête. Le test de relecture fournit directement un événement modifié à la fonction de réconciliation; il ne prétend pas contourner le contrôle d’intégrité du journal persistant. Aucun test Go, AgentOS, runtime LLM, service externe, secret réel, coût, instance de production ou changement Forgejo n’a été utilisé. Aucun code Forgejo ou GitSpace n’est copié.

Rust/Cargo `1.95.0`; les sources et leurs SHA-256 sont dans `source-sha256.txt`. Les logs rouges/verts, la lecture Cargo, les tests du garde et le manifeste local figurent dans ce dossier. Projet global toujours incomplet : pas de parcours LLM, pas de comparaison contrôlée de valeur et la course de fusion côté serveur reste ouverte.
