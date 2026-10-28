# CONTINUATION-351 — readback processus des assignees de pull request

Date : 2026-10-28. Worktree : `local/WEBHOOK-005-canonical-links`, HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. Ce lot ajoute un test et son entrée au runner allowlisté; aucune logique de production n’a été modifiée.

## Couverture ajoutée

Avant ce lot, le test processus des assignees traversait le receiver et le CLI avec les routes d’issues, tandis que le code partagé prend également en charge `ResourceKind::PullRequest`. Le scénario `actual_reconcile_cli_process_projects_current_pull_request_assignees_from_api` signe et admet une notification `pull_request/synchronized` dont la liste d’assignees est périmée, puis démarre le vrai binaire `forgejo-reconcile-local` contre une réponse HTTP fixture pour la PR 47.

Le processus exécute un GET sur `/api/v1/repos/forge-admin/agent-forge-core/pulls/47`; il retourne les assignees courants triés par ID, avec `source=current_forgejo_resource_api`, `freshness=current_api_read` et `trust=untrusted_forgejo_identity`. Le test confirme aussi le binding `kind=pull_request`, les SHA base/head courants, un seul read API, l’absence d’en-tête Authorization, l’absence de l’identité périmée dans la sortie et l’identité octet pour octet du journal avant/après.

La fixture est servie sur loopback dans Bubblewrap avec `--unshare-net` : elle ne contacte pas d’instance Forgejo. Aucun AgentOS, LLM, secret ou service externe n’est utilisé.

Commande reproductible :

```text
python3 scripts/run_webhook_reconcile_cli_e2e.py --scenario actual_reconcile_cli_process_projects_current_pull_request_assignees_from_api
```

Résultat archivé dans [`isolated-test.log`](isolated-test.log) : code 0; 1 test réussi, 0 échec, 170 tests filtrés. Le run distinct `CONTINUATION-350` couvre les assignees d’issues, le `null` inconnu et le tableau vide connu; il n’a pas été rejoué.

## Vérifications

Environnement : Cargo 1.95.0, rustc 1.95.0, Python 3.12.3, Bubblewrap 0.9.0, Linux aarch64. La sortie complète de Cargo se trouve dans [`cargo-all-targets.log`](cargo-all-targets.log).

| Commande | Résultat |
| --- | --- |
| `cargo test --offline --locked --all-targets -- --test-threads=1` | Code 0; 38 groupes, 735 réussis, 0 échec, 163 ignorés. Le test processus ajouté est ignoré par Cargo et exécuté séparément par Bubblewrap. |
| `cargo fmt --all -- --check` | Code 0 après formatage. |
| `git diff --check` | Code 0. |
| `python3 scripts/verify_cargo_license_state.py` | Code 0; aucune licence ou licence-file sélectionnée, `publish=[]`, `no_license_selected`, droits `not_assessed`, marqueur `NOASSERTION`. |
| `PYTHONDONTWRITEBYTECODE=1 python3 -m unittest -q scripts/test_cargo_license_state.py` | 12 réussis, 0 échec. |

## Runtime d’agent et limites

Réexamen ciblé : le module `src/agent_runtime.rs` définit un contrat seulement; `README.md` indique qu’aucun adaptateur de production n’est fourni. `tests/agent_runtime_contract.rs::FixtureRuntime` est un fixture déterministe, sans LLM. Cette tranche reste une vérification de la couture receiver → journal → CLI → fixture HTTP pour le type pull request. Elle ne démontre ni une instance live, ni un PR de changement de code, une vérification indépendante, une fusion, un LLM/AgentOS ou un gain mesuré. La mission reste incomplète.
