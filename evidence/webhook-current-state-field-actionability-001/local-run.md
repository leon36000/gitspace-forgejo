# WEBHOOK-CURRENT-STATE-FIELD-ACTIONABILITY-001 — diagnostic du snapshot Forgejo

Date : 25 octobre 2026. Dépôt produit `local/WEBHOOK-005-canonical-links`, HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. Le worktree est fortement modifié; ce lot n’a ni nettoyé, ni réinitialisé, ni commité les travaux.

## Lacune et modification

Pour HTTP 200 avec un snapshot incomplet/invalide, le CLI refusait correctement de publier `canonical_state` et rendait `verdict=inconclusive`, `error.code=invalid_current_state`, `retryable=false`. Toutefois, `error.details` était vide : l’agent ne pouvait pas distinguer une date invalide d’un SHA absent.

Le réconciliateur émet maintenant un champ fixe dans `error.details.field` pour `number`, `state`, `updated_at`, `html_url`, `merged`, `head.sha`, `base.sha` et l’incohérence merged/state. Le code d’erreur, le verdict et sa non-réessayabilité restent identiques. Les valeurs du JSON Forgejo, le corps HTTP et les en-têtes ne sont jamais incorporés au champ. Le champ enrichit `details`; la version de schéma CLI reste inchangée.

## Test rouge / vert

Le test processus existant a été renforcé pour exiger `details.field=updated_at`. Son premier passage avant l’implémentation a échoué avec `null` à la place de `updated_at`. Le contrôle négatif archivé dans [red-mutation.log](red-mutation.log) supprime temporairement la sérialisation du champ : le processus échoue bien, ce qui prouve que l’assertion détecte cette régression. La source a été restaurée depuis une copie byte-à-byte avant le run vert final.

Le parcours testé est webhook signé synthétique → vrai receiver local → journal durable → vrai CLI → fixture HTTP loopback renvoyant HTTP 200 avec une date impossible. Bubblewrap exécute le scénario dans un espace réseau non partagé; aucun service externe ni credential n’est utilisé.

## Vérifications reproductibles

Versions : [tool-versions.txt](tool-versions.txt).

- `python3 scripts/run_webhook_reconcile_cli_e2e.py --scenario actual_reconcile_cli_process_refuses_success_http_with_invalid_timestamp` — 1/1, voir [process-e2e.log](process-e2e.log).
- `cargo test --offline --locked --test webhook_reconcile_contract` — 23/23, voir [reconcile-contract-tests.log](reconcile-contract-tests.log).
- `cargo test --offline --locked --bin forgejo-reconcile-local` — 110/110, voir [reconcile-cli-tests.log](reconcile-cli-tests.log).
- `cargo fmt --all -- --check` et `git diff --check` — réussis, sorties [rustfmt.log](rustfmt.log) et [diff-check.log](diff-check.log).
- Le readback Cargo est [cargo-license-state.json](cargo-license-state.json) : `license=null`, `license_file=null`, `publish=[]`, décision `no_license_selected`, droits `not_assessed`.
- Les empreintes des quatre sources modifiées et des deux documents canoniques sont dans [source-hashes.txt](source-hashes.txt); le manifeste local est [SHA256SUMS](SHA256SUMS).

## Portée et limites

**Démontré :** la précision du champ dans une réponse processus réelle du CLI, la non-régression d’un code/verdict non-réessayable, et les contrats unitaires couvrant plusieurs champs. **Non démontré :** réponse de Forgejo live, PR ou fusion, agent LLM, comportement après fusion, benchmark conventionnel ou réduction générale des interventions/coûts. Une fixture synthétique ne prouve pas la qualité globale des données Forgejo. Aucun nouveau composant, dépendance ou service n’est ajouté; la licence produit n’est pas choisie.
