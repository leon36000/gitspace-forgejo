# CONTINUATION-080 — lecture canonique des jobs Forgejo Actions

Date : 26 octobre 2026. Produit : worktree `/home/pc6/Documents/Codex/2026-10-21-mission-construire-une-forge-con-ue/.runtime/webhook-005-workspace`, branche `local/WEBHOOK-005-canonical-links`, HEAD de base `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. Le checkout était déjà fortement modifié; aucun commit n’a été créé et les changements précédents ont été conservés.

## Source amont et contrat

La source primaire locale est Forgejo officiel v17.0.0 au commit `0d791421f85ca13a63b665053d81ea765fbbc571`, dans `/tmp/forgejo-v17-base-cas-poc`. Le scratch dans son ensemble reste modifié par d’autres POC; les sept fichiers cités sont propres par rapport au commit audité (`git diff --quiet HEAD -- <fichiers>` passe). Leurs empreintes SHA-256 et les versions des outils sont consignées dans `source-sha256.txt` et `tool-versions.txt`.

- `modules/webhook/type.go:46-52` et `104-116` déclarent les sept événements `workflow_job_*` et leur correspondance de statut.
- `modules/structs/hook.go:550-562::WorkflowJobPayload` contient l’action, le job et son run parent.
- `modules/structs/action.go:14-43::ActionRunJob` expose les IDs, statut, URL, étapes et métadonnées de runner.
- `services/webhook/notifier.go:989-1088` construit les payloads des trois actions et mappe les statuts.
- `routers/api/v1/repo/action.go:1619-1700::GetActionJob` lit le job par ID interne, contrôle le dépôt et retourne les étapes; `1775+::GetActionJobLogs` est une route séparée.
- `services/convert/action.go:69-117::ToActionRunJob` convertit le job et ses étapes.
- `models/actions/run_job.go:78-116::HTMLURL` construit le lien UI depuis le run, l’index du job et la tentative.

Le code produit ajoute les sept événements au contrat de capacités versionné. L’ingestion lie le suffixe d’état du header non signé au `job.status` dans le corps signé; elle refuse avant append un statut différent, un ID nul ou une liaison `job.run_id != run.id`. La réconciliation autorise uniquement deux GET exacts vers le dépôt configuré : `/actions/jobs/{job_id}`, puis `/actions/runs/{run_id}`. Elle valide les IDs et le dépôt courant, puis le lien job sous l’URL canonique du run. La route logs, les écritures, les variantes, les autres dépôts et les paramètres de requête ne passent pas l’allowlist.

La projection renvoie le statut courant du job et son URL. L’horodatage et le SHA viennent du run parent courant, car le DTO de job n’a pas d’horodatage. Elle omet les noms du job/workflow, `needs`, les étapes, runner/task IDs, logs et payload. Le statut d’un job Actions demeure une observation du runner, jamais une preuve indépendante ou un droit de fusion.

## Résultats

Le test de processus `actual_reconcile_cli_process_reads_workflow_job_and_parent_run_without_runner_details` utilise le receiver et le CLI Rust réels, un journal HMAC temporaire et une fixture HTTP loopback. Le processus test s’exécute sous Bubblewrap 0.9.0 avec `--unshare-net`, `--unshare-pid`, environnement effacé, arbre source en lecture seule et registry Cargo copiée pour la compilation offline.

Le webhook synthétique signé porte `workflow_job_failure`, ID job `9001`, run `7001` et index Web `41`. Le serveur fixture répond d’abord avec le job courant `success`, puis le run parent courant `failure`, SHA `bbbb…bbbb` et date `2026-10-26T11:05:00Z`. Résultat observé : receiver HTTP 202; deux GET, dans l’ordre exact job puis run; aucune lecture `/logs`; verdict `current_state_read`; la projection décrit l’état du job (`success`) et utilise le SHA/date du parent. Les marqueurs privés du webhook et des deux réponses API sont absents de stdout; l’inbox reste octet pour octet identique. Reçu : `process-e2e.log`, 1 réussi et 95 filtrés.

Autres vérifications :

- Tests ciblés : admission webhook 2/2; contrat de réconciliation 2/2; allowlist/adaptateur CLI 2/2.
- `cargo test --offline --locked --all-targets -- --test-threads=1 --quiet` : 596 réussis, 0 échec, 88 ignorés, répartis sur 34 exécutables de tests (`all-targets.log`).
- `cargo fmt --all -- --check`, `git diff --check`, compilation Python du runner isolé : passent.
- `python3 scripts/verify_cargo_license_state.py` et `python3 -m unittest -v scripts.test_cargo_license_state` : passent, 9 tests. Cargo reste sans sélection de licence (`license=null`, `license_file=null`, `publish=[]`, `no_license_selected`, droits `not_assessed`, marqueur `NOASSERTION`).
- Le manifeste local vérifie 20 artefacts; les manifestes append-only du produit et du dépôt parent vérifient chacun 36 chemins courants, avec leurs préfixes antérieurs conservés. Le rapport de commande du générateur est exclu de sa propre somme pour éviter l’auto-référence et est couvert par un fichier SHA-256 séparé.

## Portée et limites

`n=1`; l’API est simulée sur loopback, et aucun événement n’est émis par une instance Forgejo dans ce lot. Aucun runner Actions n’exécute de workflow; aucune qualité de job, aucun artefact ou log n’est vérifié. La séparation Bubblewrap s’applique au processus de test sur cet hôte et ne constitue pas une preuve de sécurité générale. Aucun LLM, AgentOS, RAG, mémoire, coût fournisseur, secret externe, publication ou environnement de production n’a été utilisé. Aucun code Forgejo n’est copié. Les droits de redistribution du produit restent non évalués.

Ce lot ajoute une lecture canonique des jobs; il ne ferme aucun des critères globaux ouverts : parcours réel avec runtime agent autorisé, comparaison contrôlée Forgejo+agent+CI, inventaire exhaustif/licences, ou couverture de chaque refus et défaillance. `workflow_job_success` n’est pas transformé en approbation. Voir aussi [checkpoint](../../docs/checkpoint.md), [architecture](../../docs/architecture.md), [inventaire des sources](../../docs/source-inventory.md) et [matrice des refus](../../docs/refusal-evidence-matrix.md).
