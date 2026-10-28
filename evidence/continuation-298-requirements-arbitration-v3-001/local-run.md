# CONTINUATION-298 — admission des exigences v3 via PR revue

Date : 27 octobre 2026. Travail local dans le worktree produit très modifié, base `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. Aucun reset, commit, déploiement, accès distant ou nouvel appel de modèle. Aucun code Forgejo/GitSpace copié. L’état Cargo reste sans choix de licence et sans évaluation des droits de redistribution.

## Contrat ajouté

`Requirements` accepte maintenant des champs facultatifs rétrocompatibles pour la PR source et une liste structurée de décisions (`ambiguity`, `resolution`, `rationale`). Le chemin distinct `admit_reconciled_issue_v3` refuse la sélection v3 par l’admission v2, relit l’issue ouverte et la branche protégée, valide les exigences v3 et son inventaire, compare exactement les octets de `requirements/<ID>.json` à la tête protégée et au commit de fusion de la PR indiquée, et exige une politique hôte versionnée dont les octets correspondent au SHA-256 attendu.

Le port `MissionArbitrationSourceV1` fournit la PR source et son relevé de revues. Le gate vérifie le dépôt/PR, `merged`, la branche de base, les SHAs complets, les avis liés au head exact, `APPROVED`, `official`, non `stale`, non `dismissed`, l’identité d’un compte de l’allowlist, l’absence d’IDs dupliqués et l’exhaustivité (`total_count == reviews.len()`). Le contrat du port demande aux implémentations HTTP de récupérer toutes les pages, d’obtenir `total_count` depuis `X-Total-Count` et de lier chaque réponse à la requête hôte. Le gate relit la PR avant/après les revues et effectue deux lectures complètes de celles-ci; tout changement détecté produit un refus avant allocation. Les hashes, SHA de head/merge, IDs acceptés et révision de politique sont conservés dans la requête schema 2 via `requirement_arbitration`.

Le code est dans `src/lib.rs::Requirements` et `RequirementDecisionV1`, `src/forgejo.rs::RequirementSourcePullRequestV1` et `RequirementArbitrationDecisionV1`, `src/mission_admission.rs::MissionArbitrationSourceV1`, `admit_reconciled_issue_v3`, `evaluate_requirement_arbitration` et `validate_requirement_reviews`, et `src/agent_runtime.rs::AgentExecutionRequestV1::validate`. Les références de source et empreintes sont dans [`source-sha256.txt`](source-sha256.txt).

## Vérifications exécutées

Commande ciblée finale :

```text
cargo test --locked --offline --test mission_requirement_arbitration_contract --test mission_admission_contract --test webhook_mission_admission_contract --test agent_runtime_contract
```

Résultats consignés dans [`targeted-tests.log`](targeted-tests.log) : nouveau contrat v3 **11 réussis, 0 échec**; admission v2 **19/19**; webhook v2 **15/15**; runtime **12/12** (57 tests au total). Les onze cas v3 couvrent succès avec reçu, absence de compte autorisé, mismatch des octets au commit de fusion, pagination incomplète, dérive de PR entre lectures, avis périmé/ancien head, empreinte de politique périmée, refus v3 par le chemin v2, PR non fusionnée, identité de revue dupliquée et dérive entre deux snapshots de revues. Les refus vérifient un journal de générations vide.

`cargo check --locked --offline --all-targets` passe (log [`all-target-check.log`](all-target-check.log)); `cargo fmt --all -- --check`, `git diff --check` et `python3 scripts/verify_cargo_license_state.py` passent (logs/fichier d’état joints). Versions : [`tool-versions.txt`](tool-versions.txt). Aucun test live de Forgejo n’a été lancé dans ce lot.

## Limites de portée et confiance

Le test utilise un port mémoire/fixture, pas `LocalLoopbackForgejoSource`, le CLI, une instance Forgejo réelle ou un reviewer humain. L’adaptateur CLI ne lit toujours pas les routes PR/revues et aucune politique réviseur host-only n’est encore chargée par un profil local. La politique est passée au nouveau chemin par son appelant; ce contrat ne prouve pas à lui seul l’emplacement ou l’identité de cet appelant. L’allowlist autorise des IDs de comptes Forgejo mais l’API v17 n’expose pas le type bot/humain. Les revues paginées n’offrent pas de snapshot atomique; plusieurs lectures détectent certains changements mais n’éliminent pas le TOCTOU après le dernier read. `review_snapshot_sha256` atteste les octets typés évalués, pas leur vérité.

Ce lot ne démontre pas une mission LLM, une fusion, une amélioration de productivité, un gain de tokens/coût, une identité humaine ni la sécurité d’une instance de production. Prochaine action : raccorder un lecteur HTTP local borné qui préserve et valide `X-Total-Count`, définir un chargement de politique sous profil hôte fixe inaccessible à l’agent, puis exercer le port contre une instance Forgejo de test effectivement disponible. Mission globale incomplète.
