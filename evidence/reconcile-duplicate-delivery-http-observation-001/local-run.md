# Observation HTTP indépendante du refus de sélection répétée — 25 octobre 2026

## Résultat

Le test processus existant lance le vrai `forgejo-reconcile-local` avec un delivery ID répété. Son observation précédente prouvait que le credential privé restait non consommé, mais `upstream_read_attempts=0` provenait du compteur applicatif. Cette tranche ajoute un serveur loopback indépendant sur `127.0.0.1:3130` et capture uniquement la ligne de requête, sans enregistrer l'en-tête d'autorisation.

Le test-first est exécuté avec la garde d'unicité temporairement retirée. La fixture observe exactement `GET /api/v1/repos/forge-admin/agent-forge-core/issues/41 HTTP/1.1`; le test échoue sur cet appel, ce qui démontre que le processus atteignait le réseau API avec le credential synthétique.

Après restauration de la garde, le même processus CLI renvoie `duplicate_delivery_selection`; pendant une fenêtre d'observation de 300 ms, la fixture ne reçoit aucune requête. Le test confirme aussi que le credential reste intégralement lisible par son propriétaire de test, que stdout ne le révèle pas et que l'inbox reste octet pour octet inchangée. La fixture 404 n'est utilisée qu'en cas de requête inattendue pour permettre à un ancien CLI de terminer.

## Vérifications

- [process-test-red.log](process-test-red.log) — rouge attendu; l'observateur capture le GET avec la garde retirée.
- [process-test-green.log](process-test-green.log) — 1/1 réussi avec la garde restaurée; zéro connexion observée.
- [all-targets.log](all-targets.log) — `cargo test --offline --locked --all-targets -- --test-threads=1`: 526 réussis, 0 échec, 65 ignorés dans 34 groupes. Le scénario processus ignoré dans Cargo est exécuté séparément par le runner isolé.
- [license-state.json](license-state.json) et [license-tests.log](license-tests.log) — état Cargo vérifié et garde 7/7: `license=null`, `license_file=null`, `publish=[]`, `no_license_selected`, droits de redistribution `not_assessed`.
- [rustfmt.log](rustfmt.log) et [diff-check.log](diff-check.log) — `cargo fmt --all -- --check` et `git diff --check` passent.
- Les empreintes locales et l'index global append-only `evidence/SHA256SUMS` sont recalculés/complétés puis vérifiés après la mise à jour de ce rapport.

Le processus de test et sa fixture tournent dans Bubblewrap avec `--unshare-net`; l'interface loopback privée reste accessible, tandis que le réseau externe est isolé. Aucun appel au vrai Forgejo, service externe, modèle ou credential réel n'est effectué. `Cargo.toml` reste sans champ `license` ou `license-file`, et `publish = false`; aucun choix de licence ni droit de redistribution n'est inféré.

Versions, état Git, commandes et empreintes des sources finales : [tool-versions.txt](tool-versions.txt) et [provenance.json](provenance.json).

## Portée et limites

Cette preuve complète l'observation de la frontière de credential par une mesure HTTP indépendante pour le cas de sélection répétée. Elle ne démontre pas que toutes les autres refus de réconciliation évitent les appels réseau; leurs tests doivent être appréciés séparément. Elle n'ajoute pas de comportement de production : la garde et son refus structuré proviennent du lot antérieur [RECONCILE-DUPLICATE-DELIVERY-SELECTION-001](../reconcile-duplicate-delivery-selection-001/local-run.md). Les suites `--all-targets` ignorent 65 cas processus et ne couvrent pas les doctests; un test processus ciblé a été exécuté hors de cette suite.

Le worktree reste fortement modifié, au HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`; aucun fichier n'est stage ou commité. Aucun code Forgejo du scratch n'est copié. Le parcours AgentOS, la mesure comparative appariée et l'usage LLM autorisé restent des critères ouverts; le projet n'est pas achevé.
