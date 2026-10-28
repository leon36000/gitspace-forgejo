# CONTINUATION-394 — distinguer un tag non pris en charge d’un payload invalide

Date : 28 octobre 2026. Worktree `local/WEBHOOK-005-canonical-links`, HEAD de base `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`; l’arbre de travail préexistant est conservé, sans commit, staging ou modification distante.

## Lacune

Les capabilities v8 décrivent déjà les webhooks de branche `create`/`delete` avec la contrainte `ref_type is branch`. Un payload Forgejo valide pour un tag passait la vérification HMAC puis tombait dans `ReconcileError::UnsupportedEvent`, que l’ingress transformait en `InvalidEventPayload`. Résultat observé avant la correction : HTTP 400 `event_payload_invalid`, sans `field_path` ni action utile. Cela mélangeait une donnée mal formée avec un événement valide hors des capacités.

## Changement

- Ajout de `UnsupportedEventPayloadField("ref_type")` à la classification du reconciler pour les tags `create` et `delete`.
- L’ingress garde ce cas distinct et répond HTTP 422 `unsupported_event`, non réessayable, `details.field_path=payload.ref_type`; l’action demande un filtre de confiance côté émetteur ou, si ce n’est pas disponible, la désactivation de ces événements jusqu’au support tag explicite. Le payload refusé n’est jamais journalisé.
- La relecture CLI sérialise aussi le champ et le lien vers `--capabilities`. Un événement tag déjà présent dans un ancien journal peut être quarantiné avec le motif `unsupported_event_payload_field`, champ `ref_type`, sans état canonique ni accès API.
- Aucun support tag ni changement du schéma de capacités v8 n’est ajouté.

## Preuves rouge/vert

1. `receiver-tag-red.log` est un premier échec de harnais conservé : l’inbox temporaire n’existait pas après le GET de capacités. Il ne constitue pas une preuve produit.
2. `receiver-tag-red-002.log` est le rouge produit : `python3 scripts/run_webhook_reconcile_cli_e2e.py --scenario actual_receiver_reports_tag_create_as_unsupported_payload_field_without_journaling` échoue parce que le receiver renvoie 400 `event_payload_invalid` au lieu de 422 `unsupported_event`.
3. `receiver-tag-green-003.log` est le résultat final du même processus sous Bubblewrap `--unshare-net --unshare-pid`; il lit la capacité v8, refuse le tag avec champ/action attendus et confirme que l’inbox n’a pas changé : 1/1. `receiver-tag-green-002.log` conserve le résultat antérieur avant le raffinement de l’action.
4. `cargo test --offline --locked --test webhook_handler_contract forgejo_branch -- --nocapture` : avant le changement, les deux cas tags create/delete échouent sur 400 au lieu de 422; après, 2/2 passent, avec journal vide.
5. `cargo test --offline --locked --test webhook_reconcile_contract tag_ -- --nocapture` passe 2/2; les tags create/delete sont refusés avant tout appel API.
6. Le test CLI `unsupported_event_payload_field_names_the_unsupported_ref_type` passe 1/1.
7. `quarantine-tag-red.log` est le rouge de classification de quarantaine : sans le mapping du motif, le CLI ne peut pas mettre l’ancien événement tag en quarantaine. Après restauration du mapping, `quarantine-tag-green.log` passe 1/1 et vérifie `delivery_quarantined`, le motif/champ, zéro lecture upstream et deux lignes journalisées (source originale + disposition).
8. `receiver-tag-action-red.log` a ensuite montré que la première action invitait à une configuration Forgejo supposée filtrer les tags. Le test exige maintenant un conseil sans cette hypothèse : filtre de confiance côté émetteur ou désactivation des subscriptions create/delete jusqu’au support tag explicite. Le résultat final est dans `receiver-tag-green-003.log`.

Le premier `cargo test --offline --locked --all-targets -- --test-threads=1` a aussi révélé une fixture distincte `issue_comment` qui n’avait pas migré vers le contrat v8 et omettait `is_pull`. Cette fixture attendait 202 mais recevait 400 avant d’atteindre son scénario de budget. Après ajout de `is_pull=false`, le scénario exact passe; le premier log est conservé dans `cargo-all-targets.log`, le run après réparation dans `cargo-all-targets-002.log`, et la suite finale après le raffinement de l’action dans `cargo-all-targets-003.log`.

## Vérification finale

`cargo test --offline --locked --all-targets -- --test-threads=1` dans `cargo-all-targets-003.log` : 38 groupes, 751 réussis, 0 échec, 186 ignorés. Le test receiver-process ignoré par défaut est en plus exécuté séparément sous Bubblewrap et passe 1/1. `cargo fmt --all -- --check`, `git diff --check` et `python3 scripts/verify_cargo_license_state.py` passent. Le contrôle Cargo retourne `license=null`, `license_file=null`, `publish=[]`, `no_license_selected`, droits `not_assessed`, SPDX `NOASSERTION`.

Outils : Rust/Cargo 1.95.0, Bubblewrap 0.9.0; Cargo offline et `--locked`. L’E2E emploie le vrai processus receiver local, une fixture HTTP loopback et un secret HMAC synthétique; il n’emploie pas un Forgejo vivant. Aucun LLM/AgentOS, coût modèle, réseau externe, courriel, GitLab ou production n’a été utilisé.

## Identité des sources de test

Ces empreintes identifient les fichiers complets du worktree sale à la fin du run, pas des patches isolés; plusieurs contiennent des changements antérieurs à CONT-394.

```text
fc365ae456b1e8bc94038f0ce4612f0075a399f68e2acd10717ff7b4a604b106  src/webhook/reconcile.rs
5bddfd079003120e56397d7c5b9137801d46883dc49240a68b1b372502516e10  src/webhook.rs
08b5f97f1bef9fb8e2245ac740d1e1ff774728aa166be2937c0a03e6c84979b6  src/webhook/http.rs
42b7dd178269b17b1644f4e61734607c5dcbd92c2f8fbc9fee95e65adeb3ef78  src/bin/forgejo-reconcile-local.rs
ec01e7a4ef99cc2204c1926b32f5be2266a671abc69aa7abbebaac9e4fae5724  tests/webhook_handler_contract.rs
9a1ce64844c54166c2a49720f8e7b7c6fd5bbe44271d0c0c8c5efa66f9e7cf00  tests/webhook_reconcile_contract.rs
963057c2841685995ac67d3d57c42f3f168bd827bf595659754e878d3e275ab0  tests/webhook_quarantine_cli.rs
e4fbd67657cb6e4025a54ae74c82cca9e516695e0f79de4934f1ffe55a25b837  tests/webhook_reconcile_pipeline.rs
1fe6717b5043131cc9973a903a395bf856081b75f1724698ab22e0abff44e92d  tests/webhook_receiver_process_e2e.rs
```

## Limites

Le processus receiver exerce seulement le tag `create`; le tag `delete` est couvert par les tests handler/reconciler sans sous-processus dédié. Cela ne démontre ni l’émission native d’un webhook par Forgejo ni le cycle de vie des tags. Les tags restent explicitement non pris en charge. Pas de PR/fusion ou de comparaison contrôlée; le projet est incomplet.
