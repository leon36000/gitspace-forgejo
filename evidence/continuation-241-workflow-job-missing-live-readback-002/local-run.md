# CONTINUATION-241 — 404 live du readback d’un job Actions Forgejo

Date : 27 octobre 2026. Worktree préexistant à HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`, fortement modifié. Aucun reset, commit ou nettoyage.

## Résultat

Le binaire local Forgejo `17.0.0+gitea-1.22.0` (`e5ec437cd6357e8d1b4580cb47ab1daa58a32b33971a63db63446a535d33adb7`) a démarré une instance neuve sur `127.0.0.1:3130`, dépôt privé, Actions/SSH/inscription désactivées. Un lecteur API non-admin a été provisionné. L’API Forgejo réelle retourne 404, avant et après le scénario, pour `GET /api/v1/repos/forge-admin/agent-forge-core/actions/jobs/9001`.

Le vrai receiver Rust admet la troisième livraison, synthétique et HMAC-signée, `workflow_job_success/job_completed`. Le vrai CLI reçoit le credential read-only par pipe privé et rend `inconclusive/resource_not_found`, `retryable=false`, `upstream_read_attempts=1`. `error.details.resource` conserve `kind=workflow_job`, `job_id=9001`, `run_id=7002`, `index_in_repo=42`. Il n’y a pas de `canonical_state`, pas de mutation des octets du journal, et l’action avertit que l’absence et la visibilité privée sont indiscernables avec HTTP 404. Le produit cible le job avant le run parent dans `src/bin/forgejo-reconcile-local.rs`; le résultat d’un seul readback est cohérent avec ce chemin. Le test processus CONT-081 sur fixture vérifie séparément qu’un job 404 ne déclenche pas la lecture de son parent.

Dans la même instance, le contrôle issue lit l’état courant `open` malgré un payload périmé `closed`, l’événement workflow_run manquant CONT-240 demeure inconclusif, et un credential invalide est refusé. Le run complet termine `verdict=passed`; receiver et Forgejo sont arrêtés, les ports contrôlés sont absents. Sortie expurgée, provenance, compte, lectures et checksums : [reçus du run](../continuation-241-workflow-job-missing-live-readback-001/).

## Source et implémentation

La source de référence est Forgejo v17.0.0 au commit `0d791421f85ca13a63b665053d81ea765fbbc571`. `routers/api/v1/api.go:934-955` enregistre `GET /actions/jobs/{job_id}` vers `repo.GetActionJob` dans le groupe `reqRepoReader(unit.TypeActions)`. `routers/api/v1/repo/action.go:1619-1688::GetActionJob` renvoie 404 quand `GetRunJobByID` trouve `ErrNotExist`, et aussi quand le job appartient à un autre dépôt. Les deux fichiers ciblés sont inchangés par rapport au HEAD épinglé; le scratch complet contient d’autres modifications POC. Hashes et commande de lecture : [source-review.txt](source-review.txt).

Le client produit construit l’URI autorisée `/api/v1/repos/{owner}/{repository}/actions/jobs/{job_id}` et mappe 404 à `ApiReadError::NotFound`, puis `resource_not_found`. Aucun code Forgejo/Rust produit n’a été copié ou modifié dans cette tranche; seul le runner E2E local a ajouté le scénario et l’en-tête `workflow_job_success` à son allowlist fixe.

## Vérifications et statut de licence

Le scénario live passe une fois (`n=1`) sur l’instance locale. `cargo test --offline --locked --all-targets --no-run` compile les cibles sans exécuter tous les tests; `cargo fmt --all -- --check`, `git diff --check`, parse AST Python, manifeste du run 001, `cargo metadata --locked --offline --no-deps` et `scripts/verify_cargo_license_state.py` passent. Les commandes, sorties et versions sont dans les fichiers voisins.

Le statut Cargo reste volontairement sans licence : `license=null`, `license_file=null`, `publish=[]`, `no_license_selected`, `redistribution_rights=not_assessed`, SPDX `NOASSERTION`. Les droits de redistribution ne sont donc pas accordés par ces métadonnées. AgentOS et un runtime modèle ne sont pas appelés; aucun LLM, coût, service externe ou donnée privée n’est utilisé.

## Limites et suite

Le job testé n’existe pas et Actions était désactivé. Le webhook est construit/signé par le harnais, pas émis par Forgejo; aucun runner ni job positif n’a été exécuté. Le 404 ne démontre pas qu’un run privé absent et un ID inexistant soient distinguables; aucune observation de production, autorisation merge, preuve indépendante, benchmark, token ou coût n’en découle. L’exécution Forgejo/receiver a lieu sous l’UID local; la compilation Cargo est l’étape sous Bubblewrap sans réseau.

La mission reste incomplète. Prochaine étape : choisir un autre cas Forgejo/API non couvert et non équivalent à ces deux 404; ne pas activer Actions/runner sans une isolation réellement autorisée et vérifiée.
