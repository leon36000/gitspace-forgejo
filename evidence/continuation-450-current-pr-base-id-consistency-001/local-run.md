# CONTINUATION-450 — IDs du dépôt base PR cohérents en réconciliation — 28 octobre 2026

## Décision et changement

`validate_current_pull_request_base_repository` contrôlait l’identité textuelle de `base.repo`, mais ignorait les champs numériques `base.repo_id` et `base.repo.id`. Le test rouge reproduit l’acceptation de 18 contre 17. Le lecteur descriptif vérifie maintenant chaque ID présent comme entier signé positif; si les deux sont présents, ils doivent être égaux. Une réponse partielle avec `base.repo` absent/null reste permise, sans déclarer de liaison numérique. Une divergence renvoie `ResourceMismatch`; une valeur de type incorrect ou non positive renvoie `InvalidCurrentStateField` avec le chemin du champ.

Les capacités sont versionnées à schema 31 / `forgejo-reconcile-capabilities-v31` et publient explicitement la règle, y compris l’absence de revendication numérique quand le dépôt imbriqué manque. Ce contrôle est descriptif; il ne fournit pas un ID cible configuré et n’autorise pas la fusion.

## Contrat amont examiné

Forgejo officiel v17.0.0 est épinglé au commit `0d791421f85ca13a63b665053d81ea765fbbc571`. `PRBranchInfo` porte `RepoID int64` et `Repository *Repository`; `Repository.ID` est un `int64`. `ToAPIPullRequest` remplit séparément `Base.RepoID` depuis `pr.BaseRepoID` et `Base.Repository` via `ToRepo`. Les blobs exacts, hashes, lignes et limites d’examen sont dans [upstream-source.txt](upstream-source.txt). Aucun code Forgejo n’a été copié.

## Vérifications

- TDD unitaire rouge/vert : le cas 18/17 échouait avant le changement; le test ciblé passe après correction. Les cas couvrent ID extérieur ou imbriqué nul, type chaîne, égalité, un seul ID et les réponses partielles.
- Processus composés : receiver réel, journal durable, binaire `forgejo-reconcile-local` et fixture HTTP loopback. La delivery reçoit HTTP 202; le readback 18/17 retourne `inconclusive/resource_mismatch`, sans état canonique, sans Authorization et sans changement octet pour octet de l’inbox. L’essai initial du test processus a révélé une fixture incomplète (`draft` absent), corrigée en `false`; aucun changement runtime n’a été nécessaire pour cette correction de test.
- `cargo test --locked --offline --all-targets -- --test-threads=1`: 39 groupes, 787 réussis, 0 échec, 211 ignorés. Les tests ignorés n’ont pas été exécutés par ce run.
- `cargo fmt --all -- --check`, `python3 -m py_compile scripts/run_webhook_reconcile_cli_e2e.py`, `git diff --check`, test de capacités et garde de licence : succès.
- Métadonnées Cargo : `agent-forge-core` 0.1.0, `license=null`, `license_file=null`, `publish=[]`; garde `no_license_selected`, droits `not_assessed`, SPDX `NOASSERTION`.
- Le manifeste du lot couvre 16 artefacts. Les index append-only du worktree et du projet passent respectivement avec 14 028 et 9 197 chemins vérifiés, sans entrée manquante, divergente, non hachée ou dangereuse; l’unique entrée historique non hachée du worktree reste explicitement signalée par le vérificateur.

Les sorties de commandes et environnements sont conservées dans ce répertoire; les sources sont listées dans `commands.txt`.

## Portée et limites

Le processus utilise Bubblewrap avec namespaces réseau/PID et un serveur HTTP loopback synthétique; il ne lance pas Forgejo, ne contacte pas GitLab et n’utilise aucun credential. Le test prouve le comportement du client face à une réponse contradictoire contrôlée, pas un défaut observé sur un serveur Forgejo live. Les deux IDs peuvent encore être simultanément erronés mais cohérents; l’identité textuelle reste une donnée de la même API, pas une attestation cryptographique. Aucun parcours LLM/AgentOS, comparaison de productivité, ni gain général n’est démontré.
