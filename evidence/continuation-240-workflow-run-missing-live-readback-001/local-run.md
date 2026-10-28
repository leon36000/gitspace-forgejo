# CONTINUATION-240 — refus live du readback d’un run Forgejo absent

Date : 27 octobre 2026. Source produit à HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`; dépôt fortement modifié, changements antérieurs conservés.

## Résultat final

Une instance Forgejo v17.0.0 jetable, privée et liée à `127.0.0.1:3130` a été démarrée avec Actions, SSH et inscription désactivées. Le compte API lecteur est non-admin et n’a que les permissions de lecture confirmées. Le GET réel `/api/v1/repos/forge-admin/agent-forge-core/actions/runs/7001` retourne 404 avant et après la réconciliation.

Le harnais signe un payload synthétique `workflow_run_success` / `run_completed` avec une clé locale de test. Le vrai receiver répond 202 et l’inscrit comme livraison 2. Le vrai CLI, avec credential read-only transmis par pipe privé, effectue une lecture API et rend `verdict=inconclusive`, `error.code=resource_not_found`, `retryable=false`, `upstream_read_attempts=1`. L’identité est sous `error.details.resource` (`kind=workflow_run`, `run_id=7001`, `index_in_repo=41`). L’action recommande de vérifier l’ID et l’accès, sans prétendre distinguer une absence d’un run privé masqué. Aucun `canonical_state` n’est retourné et le journal est byte-for-byte inchangé par le CLI. Le GET API de contrôle final retourne encore 404.

Dans le même run, le contrôle positif `issues` lit l’état courant `open` malgré le payload périmé `closed`; le contrôle négatif de credential reste `inconclusive/upstream_unauthorized`. Les tokens ne figurent pas dans les reçus. Les deux processus lancés par le harnais se sont terminés et les ports ont été relus absents après chaque tentative.

## Corrections du harnais et tentatives conservées

- `attempt-001.txt` : première exécution terminée avec code 1 à l’assertion du résultat absent. Le harnais attendait l’identité de ressource à la racine de la réponse au lieu de `error.details.resource`. La sortie ne contenait pas encore de reçu détaillé; arrêt des processus confirmé dans `port-cleanup-readback.json` de `...-001`.
- `attempt-002.txt` et `../continuation-240-workflow-run-missing-live-readback-002/missing-workflow-run-diagnostic.json` : le reçu expurgé démontre déjà le 404, le verdict inconclusif, la seule lecture, l’identité du run et le journal inchangé. Le code 1 était encore dû à la même assertion de test erronée. Le diagnostic ne contient pas de credential.
- `attempt-003.txt` : après correction de l’assertion vers `error.details.resource`, le parcours complet termine code 0. Son `result.json`, `provenance.json`, `port-cleanup-readback.json` et le manifeste local sont dans [le dossier du run réussi](../continuation-240-workflow-run-missing-live-readback-003/).

Ces échecs sont des défauts du nouveau harnais, pas des refus Forgejo/Rust. Aucun code produit Rust n’a été changé dans cette tranche.

## Source Forgejo et lien produit

La référence source officielle est Forgejo v17.0.0, commit `0d791421f85ca13a63b665053d81ea765fbbc571`. Dans le scratch `/tmp/forgejo-v17-base-cas-poc`, `routers/api/v1/api.go:936-951` enregistre les routes `actions/runs` sous `reqRepoReader(unit.TypeActions)`; `routers/api/v1/repo/action.go:984-1020::GetActionRun` lit le run par ID interne et répond 404 s’il est absent ou s’il appartient à un autre dépôt. Les fichiers sont inchangés par rapport au HEAD de référence, mais le scratch global est modifié par d’autres POC. SHA-256 des blobs consultés et références/symboles : [source-review.txt](source-review.txt).

Le code produit utilise la route fixe par dépôt dans `src/bin/forgejo-reconcile-local.rs` et mappe HTTP 404 en `ApiReadError::NotFound` puis `resource_not_found`; ce test ajoute un contrôle contre l’API réelle sans modifier cette logique. Binaire Forgejo local : `17.0.0+gitea-1.22.0`, Linux arm64, Go 1.26.9, SHA-256 `e5ec437cd6357e8d1b4580cb47ab1daa58a32b33971a63db63446a535d33adb7`.

## Vérifications

`cargo test --offline --locked --all-targets --no-run` compile toutes les cibles mais n’exécute pas la suite complète. `cargo fmt --all -- --check`, `git diff --check`, analyse AST Python du runner, `scripts/verify_cargo_license_state.py` et `cargo metadata --locked --offline --no-deps` passent. Les versions, commandes, sorties et empreintes sont consignées dans les fichiers voisins. Le manifeste local du run 003 passe depuis son propre dossier; le helper de ce lot vérifie les préfixes et dernières entrées ajoutées aux index append-only.

Deux probes auxiliaires de format ont d’abord échoué par surcontrainte du LF final de l’ancien runner et par un littéral tab sur-échappé dans le checker. Leurs sorties sont conservées; la vérification corrigée passe pour huit fichiers source/document (AST et whitespace) sans modifier ces fichiers.

Un contrôle global préalable `sha256sum -c evidence/SHA256SUMS` a retourné 2713 différences et une ligne mal formée. Ce contrôle sur l’index historique append-only n’établit pas l’intégrité des révisions courantes : l’audit antérieur CONT-225 avait déjà constaté des empreintes de versions précédentes et des lignes mal formées. Aucun index n’a été modifié par cette commande; le présent lot vérifie son manifeste local et les seules entrées globales nouvellement ajoutées.

Cargo reste sans licence choisie : `license=null`, `license_file=null`, `publish=[]`, `no_license_selected`, `redistribution_rights=not_assessed`, SPDX `NOASSERTION`. Aucun code Forgejo/GitSpace n’est copié et aucun service externe, modèle payant ou LLM/AgentOS n’est appelé.

## Limites et prochaine action

Une instance, un dépôt et un run absent (`n=1`). Le webhook est synthétique même s’il est signé; Forgejo ne l’a pas émis. Actions était désactivé : aucun workflow ni runner n’a été exécuté. Cette preuve observe le comportement 404 d’une route API réelle et son traitement côté reconciler, pas un verdict de CI indépendant, un gate/merge, une exécution LLM ou un gain comparatif. Le 404 n’identifie pas sa cause. Les trois répertoires d’état sous `/tmp/forgejo-cont240-live-workflow-20261027-01`, `-02` et `-03` restent privés en mode 0700; ils n’ont pas été lus ni supprimés.

La mission demeure incomplète. La prochaine étape doit choisir une lacune indépendante de la matrice et ne pas activer un runner tant qu’une isolation autorisée n’est pas démontrée; une instance avec Actions désactivées ne permet pas d’établir une émission native ou un run positif.
