# CONTINUATION-079 — lecture canonique des runs Forgejo Actions

Date : 26 octobre 2026. Worktree `/home/pc6/Documents/Codex/2026-10-21-mission-construire-une-forge-con-ue/.runtime/webhook-005-workspace`, HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`, branche `local/WEBHOOK-005-canonical-links`; checkout fortement modifié, sans commit créé.

## Source et décision

La source amont examinée est Forgejo officiel v17.0.0 au commit `0d791421f85ca13a63b665053d81ea765fbbc571`, dans `/tmp/forgejo-v17-base-cas-poc`. Les sept événements, le corps `WorkflowRunPayload { action, run }`, les trois actions d'émission, les champs DTO, l'endpoint `GetActionRun`, son binding au dépôt et l'URL Web dérivée de l'index sont référencés dans `docs/source-inventory.md`; les SHA-256 des sept fichiers amont sont dans `source-sha256.txt`. Ces fichiers sont propres par rapport au commit audité; le scratch global reste modifié par d'autres POC. Aucun code Forgejo n'a été copié et aucun binaire Forgejo n'a été démarré dans ce lot.

Le contrat produit ajoute les sept `workflow_run_*` au catalogue versionné. L'ingestion valide le statut signé `run.status` contre le suffixe du header, lequel n'est pas signé. Elle lie l'événement à `run.id`, `run.index_in_repo` et `run.repository.full_name`, puis le reconciler fait un seul GET au chemin exact configuré `/repos/{owner}/{repository}/actions/runs/{run_id}`. Le readback exige ID, index, identité du dépôt, statut connu, SHA non nul, `updated` valide et URL attendue `/actions/runs/{index_in_repo}`. La sortie ne copie pas `event_payload`, `trigger_user`, `jobs`, `logs` ou les artefacts. Le statut `success` est un signal informatif d'Actions, pas une vérification indépendante, ni une décision de fusion.

## Parcours et résultat

Le test `actual_reconcile_cli_process_reads_workflow_run_canonically_and_projects_safe_fields` traverse le receiver Rust réel, la signature HMAC, l'inbox durable, puis le CLI Rust réel. Une fixture HTTP loopback joue le readback API; Bubblewrap impose `--unshare-net --unshare-pid`, un environnement nettoyé et un workspace source monté en lecture seule. Le webhook porte un `workflow_run_success` signé, l'ID interne `7001`, l'index Web `41` et un marqueur sensible de payload. La fixture API répond `failure`, avec un SHA et une date plus récents et un autre marqueur `event_payload`.

Résultats observés : HTTP 202 à l'ingestion; exactement un GET de `.../actions/runs/7001`; `current_state_read` retourne le statut canonique `failure`, le SHA relu et l'URL de l'index `41`; les deux marqueurs sont absents de stdout; le journal reste octet pour octet identique. Le reçu final est `process-e2e.log` (1 réussi, 94 filtrés). Ce test est réutilisable via :

```sh
PYTHONDONTWRITEBYTECODE=1 python3 scripts/run_webhook_reconcile_cli_e2e.py \
  --scenario actual_reconcile_cli_process_reads_workflow_run_canonically_and_projects_safe_fields
```

Le test d'ingestion `forgejo_v17_workflow_run_delivery_is_admitted_and_bound_to_the_run` valide le binding au run; `unsigned_event_header_substitution_is_refused_when_payload_shape_disagrees` vérifie qu'un header failure ne peut pas relabeler le corps signé success. `workflow_run_success_webhook_uses_current_api_state_and_redacts_event_payload` vérifie en contrat que l'API courante prime et que la projection n'a pas de champ payload.

## Vérifications

- `cargo test --offline --locked --all-targets -- --test-threads=1 --quiet` : 590 réussis, 0 échec, 87 ignorés, 34 groupes. Sortie finale après complétion des champs du contrat : `all-targets-final2.log`.
- Processus isolé `actual_reconcile_cli_process_reads_workflow_run_canonically_and_projects_safe_fields` : 1 réussi, 94 filtrés, sous Bubblewrap 0.9.0; dernière exécution : `process-e2e-final.log`.
- Les suites dédiées `webhook_handler_contract` et `webhook_reconcile_contract` : 24/24 et 49/49; les tests du binaire CLI correspondant à `workflow_run` : 2/2. Elles sont incluses dans la suite complète et leurs résultats détaillés y figurent.
- `cargo fmt --all -- --check`, `git diff --check`, py_compile du runner, `python3 scripts/verify_cargo_license_state.py` et `python3 -m unittest -v scripts.test_cargo_license_state` passent. La vérification licence indique `license=null`, `license_file=null`, `publish=[]`, `no_license_selected`, droits `not_assessed`, `NOASSERTION`; les neuf tests passent.

Le test TDD initial a d'abord échoué avec `UnsupportedEvent`; l'extrait est conservé dans `tdd-red.log`. Deux autres retours de test ont trouvé des attendus périmés du harnais : la réponse single a été lue avec le schéma batch, puis les tests processus/capacités attendaient encore neuf événements. Après alignement au contrat single et à la liste additive de 16 événements, le parcours processus ciblé et la suite complète sont verts. Le journal initial de la première suite complète, conservé séparément comme `all-targets.log`, correspond à la liste de capacités périmée; le journal final est `all-targets-final.log`. `process-shape-first-failure.log` garde l'échec d'assertion du schéma de sortie avant correction.

## Portée et limites

`n=1`, receiver et CLI réels, serveur API simulé sur loopback. Aucun événement n'a été émis par une instance Forgejo dans ce lot; aucun test Go upstream n'a été exécuté. Cela valide la plomberie d'un readback canonique et la projection dans ce scénario, pas la fiabilité du runner Actions, les artefacts, l'isolation d'un runner Forgejo, ou un résultat CI digne de confiance. Aucun LLM/AgentOS/RAG/mémoire, coût fournisseur, credential externe, publication ou environnement production n'a été utilisé. Le benchmark apparié et plusieurs critères cumulés de la mission restent ouverts; un parcours n'établit aucun gain.
