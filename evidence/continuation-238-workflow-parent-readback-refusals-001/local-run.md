# CONTINUATION-238 — refus actionnable du readback parent workflow

Date : 27 octobre 2026. Worktree local préexistant et fortement modifié; base observée `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. Aucun reset, commit ou changement d’instance n’a été fait.

## Résultat

Une livraison `workflow_job_success` signée est admise par le receiver réel. Le CLI réel lit le job courant (`id=9001`, `run_id=7001`, statut `success`), puis son run parent. Avec une réponse HTTP 404 au second GET, il renvoie `inconclusive/resource_not_found`, `retryable=false`, sans `canonical_state`, sans marqueur du corps 404 en sortie et sans réécriture du journal. L’observation réseau de la fixture confirme l’ordre `job → run` et l’absence d’en-tête `Authorization` dans ce scénario sans credential.

Le premier essai rouge a exposé un défaut d’actionnabilité du contrat : le refus nommait seulement `workflow_job`, ressource du webhook, et son action demandait de vérifier le job, alors que le 404 venait du run parent. Le résultat distingue maintenant les deux identités : `details.resource` reste le job de l’événement; `details.readback_resource` indique `workflow_run`, `run_id=7001`, `index_in_repo=41`. L’action nomme le run parent et précise que Forgejo peut masquer un accès privé sous 404. Le code ne prétend pas distinguer suppression et visibilité.

Le scénario corrigé `actual_reconcile_cli_process_refuses_missing_parent_run_after_valid_job_readback` passe 1/1. Le scénario adjacent `actual_reconcile_cli_process_refuses_unknown_parent_run_status_after_valid_job_readback` passe aussi 1/1 après extraction du helper partagé; chacun filtre 146 autres tests. Les deux s’exécutent sous Bubblewrap avec `--unshare-net --unshare-pid`, à partir d’une copie en lecture seule du dépôt et d’une fixture HTTP loopback. Le run initial en échec, les deux exécutions corrigées et le scénario `unknown` antérieur sont conservés dans les logs de ce dossier.

## Changement et contrôles

`ReconcileError` distingue désormais le 404 du run parent; le mapping CLI donne l’identifiant de la ressource lue et le helper de projection conserve séparément la ressource d’origine. Le test processus affirme les deux champs, l’action, les deux GET ordonnés, l’absence de résultat canonique, l’absence de fuite de marqueurs et l’intégrité du journal. Le test inconnu reste un contrôle adjacent contre une régression du helper.

`cargo test --offline --locked --all-targets --no-run` compile toutes les cibles sans exécuter toute la suite. `cargo fmt --all -- --check`, `git diff --check`, la compilation Python du runner, et `scripts/verify_cargo_license_state.py` passent. Les commandes complètes, versions, hashes source et états de manifestes sont joints. Cargo reste `agent-forge-core 0.1.0`, `license=null`, `license_file=null`, `publish=[]`; décision `no_license_selected`, droits de redistribution `not_assessed` et marqueur `NOASSERTION`.

## Portée et limites

Deux exécutions processus, une ressource par scénario (`n=1` chacun), réponses API synthétiques sur loopback. Aucun serveur Forgejo ou runner Actions n’est exercé; aucune révocation réelle d’ACL, course entre readbacks, LLM/AgentOS, dépense ou benchmark n’est démontré. L’enum Rust `ReconcileError` est publique et l’ajout d’une variante peut casser un consommateur externe avec un `match` exhaustif; les consommateurs hors de ce workspace ne sont pas audités. La fixture prouve seulement la classification/action projetée pour cette réponse 404.

La mission générale demeure incomplète : le parcours comparatif contrôlé, l’agent intégré autorisé, les écarts d’upstream et plusieurs critères de sécurité/valeur ne sont pas établis par ce lot.
