# CONTINUATION-442 — lier l’issue API au dépôt configuré

Date : 28 octobre 2026. Travail local sur `local/WEBHOOK-005-canonical-links`, HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`.

## Résultat

`parse_issue_snapshot` contrôlait le numéro et `html_url`, mais acceptait un DTO qui disait `repository.full_name=other/agent-forge-core` alors que l’URL issue et le scope indiquaient `forge-admin/agent-forge-core`. Le nouveau test unitaire a échoué avant correction avec `Ok(IssueSnapshotV1)` au lieu de `Err(InvalidResponse)`.

Le parseur exige maintenant l’objet `repository`, un `owner` chaîne, `name` et `full_name`; ces trois champs doivent correspondre au scope fixe, en plus du numéro et de l’URL canonique. Les tests refusent les métadonnées absentes, mal formées, à propriétaire contradictoire et à nom divergent. Les fixtures positives reflètent la forme de l’API.

Forgejo officiel v17.0.0, commit `0d791421f85ca13a63b665053d81ea765fbbc571`, expose dans `GET /repos/{owner}/{repo}/issues/{index}` `Issue.repository` avec `RepositoryMeta.owner` sérialisé en chaîne, `name` et `full_name`. La trace route → convertisseur → DTO, les Git blob IDs et les limites de l’examen sont consignés dans [l’audit source ciblé](../forgejo-v17-issue-repository-meta-process-001/upstream-route-followup.md). Aucun code Forgejo n’a été copié.

## Vérifications

- Le run rouge du test ciblé a échoué comme prévu avant l’implémentation.
- `cargo test --locked --offline --bin forgejo-mission-local -- --test-threads=1` : **49 réussis, 0 échec, 0 ignoré**.
- `python3 scripts/run_forgejo_mission_delivery_process_e2e.py --only actual_mission_cli_refuses_issue_snapshot_change_during_admission_then_retries_current_state` : **1/1 réussi** sous Bubblewrap, réseau et PID non partagés. Le scénario accepte le DTO valide, détecte le changement d’issue durant l’inventaire avant allocation/handoff, puis le retry du même delivery réclame la génération 1.
- `cargo fmt --all -- --check` et `git diff --check` passent.
- `python3 scripts/verify_cargo_license_state.py` passe; `license=null`, `license_file=null`, `publish=[]`, `no_license_selected`, droits `not_assessed`, SPDX `NOASSERTION`.
- `append_manifests.py` a terminé avec code 0 : le préfixe des deux index est inchangé et les deux vérificateurs passent. Le premier essai du script s’est arrêté avec une erreur de type après avoir créé seulement le reçu de préfixe et un manifeste local vide; aucun index global n’avait été modifié. Le script a été corrigé pour reprendre uniquement si ce préfixe est toujours exact et si le manifeste local est vide, puis relancé avec succès.
- [append_receipt_revision.py](append_receipt_revision.py) consigne ensuite les hashes finaux du rapport, des commandes et de ce helper lui-même en nouvelles lignes, puis revalide les deux index et leurs préfixes.

Lors de la première exécution processus, le scénario s’est arrêté sur `mission_source_invalid_response` au GET de l’arbre : sa fixture omettait le champ racine `sha`, pourtant requis par `parse_requirement_tree_page`. La fixture a été corrigée pour refléter ce contrat, puis le scénario complet a passé. Cette observation ne constitue pas un défaut de runtime.

Le runner final a enregistré `source_tree_sha256=6a90488bf61b2cec61a912a9ba7ba5bb1ec8eaeaf73fe532ab787100f6340c09`, Rust `1.95.0`, Cargo `1.95.0`, Bubblewrap `0.9.0`, Linux aarch64. L’extrait final est dans [process-run-summary.txt](process-run-summary.txt). Les commandes exactes sont dans [commands.txt](commands.txt), l’environnement dans [environment.txt](environment.txt), et l’échec rouge observé dans [red-test-observation.txt](red-test-observation.txt).

## Portée et limites

Le processus utilise une fixture HTTP loopback, pas un serveur Forgejo live. La vérification lie les champs textuels et la route au scope configuré; elle ne prouve ni ID numérique de dépôt, ni authenticité/TLS de la réponse, ni stabilité après la dernière lecture. Aucun LLM, AgentOS, token modèle, merge ou benchmark comparatif n’a été utilisé. Cargo demeure sans licence choisie et les droits de redistribution ne sont pas évalués. Le projet reste incomplet.
