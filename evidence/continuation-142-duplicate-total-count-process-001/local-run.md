# CONTINUATION-142 — refus processus des en-têtes `X-Total-Count` ambigus

Date : 26 octobre 2026. Branche locale `local/WEBHOOK-005-canonical-links`, base observée `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`; le worktree reste fortement modifié. L’intégration ajoutée est un test de processus seulement; aucun code de production ne change.

## Ce qui a été vérifié

Le parseur de l’adaptateur possédait déjà `single_curl_header_value`, qui refuse plusieurs occurrences d’un en-tête insensible à la casse, et un test unitaire couvrait `X-Total-Count` répété. Ce lot ferme une lacune de portée processus pour le chemin `--commit-status-snapshot` : un serveur loopback renvoie HTTP 200 et deux lignes `X-Total-Count` dont les casses diffèrent. Le vrai CLI refuse avec code 1, stdout vide et un message demandant de ne pas utiliser la réponse comme vérification. La fixture observe un GET unique vers la page 1, sans `Authorization`; aucun retry n’est observé pendant la fenêtre de calme. Le marqueur contenu dans le corps de réponse n’apparaît pas en stderr.

Commande : `python3 scripts/run_webhook_reconcile_cli_e2e.py --scenario actual_commit_status_snapshot_cli_refuses_duplicate_case_variant_total_count_headers`. Le runner compile hors ligne les binaires et la cible d’intégration dans un espace temporaire, puis exécute le processus sous Bubblewrap avec `--unshare-net --unshare-pid`. Résultat final : 1 passé, 0 échec, 0 ignoré, 118 filtrés. Sortie intégrale : [focused-run.log](focused-run.log).

Vérifications complémentaires : `cargo fmt --all -- --check`, `git diff --check`, readback `cargo metadata --locked --offline --no-deps`, `scripts/verify_cargo_license_state.py` (status `verified`), ses neuf tests, et analyse AST Python du runner. Le premier format-check a signalé trois replis de ligne dans le nouveau test; le format a été corrigé, puis le test exact et les contrôles finaux ont repassé. Les sorties sont conservées dans ce dossier.

Une première validation JSON lancée en parallèle avec la réécriture du reçu de licence a lu ce fichier pendant sa troncature et a échoué; le garde Cargo et ses tests avaient pourtant terminé avec succès. La validation JSON a ensuite été relancée après la fin des écritures et passe. Les deux sorties sont conservées pour distinguer cette course du résultat du produit.

Le readback Cargo confirme pour `agent-forge-core 0.1.0`: `license=null`, `license_file=null`, `publish=[]`, `decision=no_license_selected`, `redistribution_rights=not_assessed`, `spdx_package_license_declared=NOASSERTION`. C’est l’état technique demandé sans choix de licence; il ne constitue pas une décision du titulaire, un avis de compatibilité ou une permission de redistribution. L’inventaire hors ligne des dépendances Cargo reste limité à son lockfile et cache local, et n’évalue pas les obligations juridiques; voir [source-inventory.md](../../docs/source-inventory.md) et [l’inventaire Cargo](../cargo-dependency-license-inventory-001/local-run.md).

Reprise des interfaces locales : la seule implémentation Rust trouvée par `rg -n "impl .*AgentRuntimeV1" src tests --glob '*.rs'` est `tests/agent_runtime_contract.rs::FixtureRuntime`. Aucun AgentOS, modèle, Forge Code, RAG ou service mémoire n’est appelé dans ce lot; aucun coût ni accès externe n’est engagé.

## Portée et limites

Le test exerce le binaire CLI, son client HTTP `curl`, la sérialisation des métadonnées d’en-têtes et le refus avant projection d’un snapshot. Le serveur, les statuts et les en-têtes sont synthétiques. Il s’agit d’un seul format ambigu sur une seule route; aucun Forgejo live, runner CI, agent LLM, contrôle indépendant ou merge n’est prouvé. Le snapshot demeure consultatif et ne peut autoriser une fusion. Cette preuve ne règle pas le benchmark contrôlé ni l’intégration AgentOS/LLM, et ne généralise pas la politique de merge à tous les writers. Le projet reste incomplet.

Révision, versions et empreintes des sources pertinentes : [revision.txt](revision.txt), [tool-versions.txt](tool-versions.txt), [source-sha256.txt](source-sha256.txt). Résultat machine-readable : [scenario-result.json](scenario-result.json). Les index locaux/parent append-only sont vérifiés par [manifest-check.txt](manifest-check.txt).
