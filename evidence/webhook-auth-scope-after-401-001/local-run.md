# WEBHOOK-AUTH-SCOPE-AFTER-401-001 — résultat local

Date d’exécution : 2026-10-25. HEAD du dépôt : `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`.

## Question vérifiée

Après un `401` pour une ressource du dépôt autorisé, le latch d’authentification du client de réconciliation peut-il masquer ou reclassifier un événement d’un autre dépôt ?

## Résultat observé

Le test `actual_reconcile_cli_process_preserves_out_of_scope_refusal_after_401` traverse le récepteur réel de test, son journal local et le binaire `forgejo-reconcile-local`. Il livre une issue #41 du dépôt configuré puis une issue #42 de `untrusted-owner/other-repository`. Une fixture HTTP loopback répond `401` au premier GET. Le processus effectue exactement ce GET pour `/api/v1/repos/forge-admin/agent-forge-core/issues/41`; il n’envoie aucune requête pour le dépôt hors scope.

Le lot retourne `inconclusive`, avec `upstream_unauthorized` pour l’issue autorisée et `upstream_forbidden` pour l’événement hors scope. Le refus hors scope ne révèle pas le nom du dépôt dans `details`; aucun résultat n’a de `canonical_state`. Le credential et le marqueur synthétiques du corps 401 ne figurent ni dans stdout ni dans stderr. Les octets de l’inbox sont identiques avant et après la réconciliation. Le runner cible passe : 1 réussite, 0 échec, 0 ignoré, 46 tests filtrés.

## Périmètre et limites

Ce test confirme l’ordre de contrôle dans une fixture locale isolée sous Bubblewrap; il ne teste pas un serveur Forgejo réel, la concurrence entre clients, l’état après une fusion, une politique de protection de branche ou un agent LLM. Le lot ne modifie que le test et la liste de scénarios du runner; le code runtime ne change pas. Le test de 401/batch et son cas de reprise par credential renouvelé sont des preuves précédentes distinctes, non rejouées ici.

Le readback Cargo rapporte `license=null`, `license_file=null`, `publish=[]`, décision `no_license_selected` et droits `not_assessed`; aucune licence n’est choisie.

## Fichiers de preuve

- `commands.txt` : commande exacte du runner.
- `test-result.txt` : résultat ciblé consigné.
- `tool-versions.txt` : versions locales pertinentes.
- `cargo-license-state.json` : état de licence relu par le garde du dépôt.
- `source-sha256.txt` : empreintes des sources et documents référencés.
- `SHA256SUMS` : intégrité des fichiers de ce rapport.
