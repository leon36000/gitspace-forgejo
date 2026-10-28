# CONTINUATION-236 — collision JSON après décodage d’une clé échappée

Date : 27 octobre 2026. Worktree local modifié, conservé sans reset; HEAD de base `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`.

## Résultat

Le scénario `actual_reconcile_cli_process_refuses_escape_equivalent_duplicate_workflow_status_before_projection` traverse le vrai receiver, le journal et le vrai CLI. Le receiver accepte un webhook `workflow_run_success` signé et conforme. La fixture HTTP loopback répond à la route Actions du run avec un JSON dont le premier membre est `"status":"failure"` et le suivant est `"sta\u0074us":"success"`. Après décodage JSON, les deux noms sont `status`.

Dans Bubblewrap `--unshare-net --unshare-pid`, le parseur strict rejette ce corps avant toute projection : le CLI répond `inconclusive/upstream_invalid_response`, `retryable=false`, un seul readback et aucun `canonical_state`. La réponse ne divulgue pas son marqueur; aucun en-tête `Authorization` n’est envoyé à la fixture; l’inbox reste byte-identical. Le scénario passe 1/1, 144 tests filtrés.

## Changements et vérifications

Aucune logique de production n’a été changée. Le test processus existant couvre maintenant l’équivalence après décodage d’échappement et reste dans la liste du lanceur isolé. Le contrôle final de format, `git diff --check`, la syntaxe Python du runner et la garde de métadonnées Cargo passent. Les index append-only local, worktree et projet conservent leurs préfixes.

Cargo metadata hors réseau confirme `agent-forge-core 0.1.0`, `license=null`, `license_file=null`, `publish=[]`; `package.metadata.agent-forge.licensing` conserve `decision=no_license_selected`, `redistribution_rights=not_assessed`, `spdx_package_license_declared=NOASSERTION`. Aucune licence ou autorisation de redistribution n’est inférée.

Versions, commandes, sorties, empreintes sources et manifestes figurent dans ce dossier.

## Portée et limites

Il s’agit d’une fixture API loopback; aucun serveur Forgejo ou runner Actions n’est utilisé. Un seul champ `status`, une seule forme d’échappement, un run et une version courante du binaire sont couverts. Le test confirme l’application de la règle du parseur strict à ce chemin; il n’établit pas la validité de CI, la correction d’un projet, la couverture de toutes les surfaces JSON, ni un gain. Aucun LLM, AgentOS, RAG, mémoire, coût fournisseur, production ou service externe n’est impliqué.

La comparaison appariée, une interface AgentOS/modèle autorisée, l’exclusion de tous writers Forgejo et l’évaluation des droits de redistribution restent ouverts. Le projet demeure incomplet.
