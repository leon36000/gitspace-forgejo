# CONTINUATION-237 — refus du statut inconnu au readback du run parent

Date : 27 octobre 2026. Worktree local, préexistant et modifié, préservé; HEAD de base `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`.

## Résultat

Le scénario de processus `actual_reconcile_cli_process_refuses_unknown_parent_run_status_after_valid_job_readback` signe et fait accepter au vrai receiver un événement `workflow_job_success` valide. Une fixture HTTP loopback fournit d’abord le readback courant du job (`id=9001`, `run_id=7001`, `status=success`), puis le readback du run parent (`id=7001`, `index_in_repo=41`, `status=unknown`).

Le vrai CLI envoie les deux GET dans l’ordre job puis run parent. Après la deuxième lecture, il retourne `inconclusive/invalid_current_state`, `details.field=status`, `retryable=false`, deux lectures amont et aucun `canonical_state`. La sortie ne contient pas les marqueurs du webhook ou de la fixture; la requête n’inclut pas de credential; le journal demeure byte-identical. Le test passe 1/1 sous Bubblewrap `--unshare-net --unshare-pid` avec 145 tests filtrés.

Le statut refusé est défini par `StatusUnknown` (`"unknown"`) dans le source Forgejo v17.0.0 au commit `0d791421f85ca13a63b665053d81ea765fbbc571`; le fichier correspondant est resté identique à ce commit dans le scratch et son blob/hash sont relevés dans [upstream-source.txt](upstream-source.txt).

## Changements et vérifications

Le code de production rejetait déjà un statut non pris en charge dans `current_workflow_run_snapshot`; aucune logique de production n’a changé. Ce lot ajoute une preuve processus distincte du refus d’un job au statut inconnu avant lecture parent (CONT-234) : ici, la première lecture du job est valide, puis c’est la seconde lecture qui bloque toute projection. La matrice distingue désormais ces deux frontières.

`cargo fmt --all`, le contrôle final de format, `git diff --check`, `python3 -m py_compile scripts/run_webhook_reconcile_cli_e2e.py` et le garde Cargo passent. Readback Cargo hors réseau : `agent-forge-core 0.1.0`, `license=null`, `license_file=null`, `publish=[]`; métadonnées personnalisées `decision=no_license_selected`, `redistribution_rights=not_assessed`, `spdx_package_license_declared=NOASSERTION`. Aucun choix de licence ni droit de redistribution n’est déduit.

Commandes, versions, sorties, empreintes et manifestes append-only sont conservés dans ce dossier. Les manifestes vérifient les préfixes existants et les dernières entrées globales.

## Portée et limites

Les deux readbacks proviennent d’une fixture loopback; il n’y a ni serveur Forgejo ni runner Actions. Le test couvre un seul job, run parent et statut. Les deux GET sont séquentiels : aucune atomicité ou cohérence instantanée contre un changement concurrent entre lectures n’est prouvée. Le statut Actions n’est pas un verdict de CI. Aucun LLM/AgentOS, production, dépense fournisseur ou benchmark n’intervient.

La comparaison appariée, un runtime AgentOS/modèle utilisable et autorisé, l’autorité atomique contre tous les writers et l’évaluation juridique des redistributions restent ouverts; la mission demeure incomplète.
