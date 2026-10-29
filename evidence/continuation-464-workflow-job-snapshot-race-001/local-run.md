# CONTINUATION-464 — relire le job après son run parent

Date : 29 octobre 2026 (America/Montreal). Worktree local : `.runtime/webhook-005-workspace`; HEAD de base `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`, déjà fortement modifié. Aucun reset, nettoyage, commit local ou remplacement global de fichier.

## Défaut ciblé

`reconcile_workflow_job` lisait un job courant, puis son run parent. Le résultat associait le statut du job au SHA/date du parent sans vérifier que le job n’avait pas changé pendant la seconde requête. Les statuts du webhook ne sont pas la source d’état courant, mais les deux lectures API séparées pouvaient aussi former un état composé périmé.

## Correction

Le réconciliateur valide maintenant les champs projetés du job par un helper commun, lit le parent, puis appelle `read_current_fresh(job)`. Une transition d’identité ou de statut entre les deux lectures produit `ResourceMismatch`; une URL devenue mal formée produit `InvalidCurrentStateField`. Le wrapper `CachedCurrentState` invalide déjà la réponse de batch précédente lors de `read_current_fresh`, donc cette lecture atteint bien l’adaptateur. Les URL de job non canoniques sont rejetées dès la première lecture, avant le GET parent.

Le test-first `workflow_job_readback_refuses_status_change_during_parent_read` a échoué avant le changement de production : le job simulé était `running`, le parent répondait `success`, puis le job était devenu `success`; l’ancienne implémentation retournait l’état composé `running` + parent `success`. L’observation concise est conservée dans `test-first-observation.txt`; la sortie de ce premier run n’avait pas été redirigée vers un fichier brut.

## Vérifications

- `cargo test --offline --locked --test webhook_reconcile_contract` : **116 réussis, 0 échec, 0 ignoré**.
- `actual_reconcile_cli_process_reads_workflow_job_and_parent_run_without_runner_details` : **1/1** sous Bubblewrap avec réseau et PID non partagés, API loopback synthétique. L’ordre observé est job → run parent → job frais; `upstream_read_attempts=3`; le statut du job reste distinct de celui du run, aucun endpoint logs n’est demandé, aucun détail de runner/job privé n’est projeté et le journal signé ne change pas.
- `cargo fmt --all -- --check` et `git diff --check` : réussis.
- Le manifeste de paquet couvre 10 artefacts; les index append-only du worktree et du projet passent sans chemin absent, divergent ou unsafe. Le worktree conserve une seule entrée historique non hashée.
- `all-targets` n’a pas été relancé dans ce lot.

Les commandes, versions et sorties sont dans `commands.txt`, `contract-tests.log`, `process-e2e.log`, `rustfmt.log` et `diff-check.log`.

## Portée et limites

Le refus de transition est prouvé au contrat avec une fixture déterministe, `n=1`; le processus prouve la nouvelle lecture fraîche sur le cas stable, `n=1`. Aucun serveur Forgejo live, runner Actions, LLM ou AgentOS n’a été utilisé. Le troisième GET augmente la latence et la charge de lecture. Ce contrôle réduit la fenêtre de course mais ne fournit pas de snapshot atomique : un changement après le dernier GET reste possible. Les statuts Actions restent consultatifs et n’autorisent ni vérification indépendante ni fusion.

Le lot n’adopte ni code Forgejo/GitSpace, ni licence, et n’engage aucun coût de modèle. Voir [la ligne CONT-464 de la matrice](../../docs/refusal-evidence-matrix.md) et [le checkpoint](../../docs/checkpoint.md). La mission globale reste incomplète.


## Synchronisation et readback GitLab PC4

Le commit `50ef4b9327204dd787cb84ff5bb94074a8c2c18d` synchronise le lot sur `codex/pc6-transfer-work-2026-10-28`. La branche est toujours inscriptible, non protégée et non par défaut; le SHA distant correspond au commit. Le diff GitLab rapporte 17 fichiers, 729 ajouts et 40 suppressions. Le readback confirme à l’identique les 11 fichiers de preuve créés et les 187 967 caractères du fichier `tests/webhook_reconcile_contract.rs`; les nouvelles sections source/test, matrice, checkpoint et manifeste sont présentes. Détails et chemins vérifiés : `remote-readback.json`. Le commit utilise `[skip ci]`; aucune pipeline n’a été lancée.

Cela synchronise des contenus de fichiers via l’API GitLab. Le graphe Git PC6, les archives binaires et un runner/exécuteur sur PC4 ne sont pas transférés ou démontrés; les tests de ce lot ont été exécutés localement sur PC6.
