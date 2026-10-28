# CONTINUATION-239 — refus du run parent conservé sur un lot de jobs répétés

Date : 27 octobre 2026. Worktree local préexistant, basé sur `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`; les changements existants sont conservés.

## Résultat

Deux IDs de livraison distincts portent le même événement `workflow_job_success` et identifient le même job `9001` et run parent `7001`. Le receiver réel accepte les deux livraisons. Le vrai CLI de lot lit le job courant une fois (`success`), puis lit le run parent une fois, qui renvoie 404. Il termine en code 1 avec un batch `inconclusive` et deux tentatives de lecture amont. Chacun des deux résultats est lié à son ID de livraison et conserve `resource_not_found`, `retryable=false`, `details.resource.kind=workflow_job` ainsi que `details.readback_resource.kind=workflow_run`, `run_id=7001`. L’action nomme le parent.

Le fixture reçoit exactement deux GET dans l’ordre `actions/jobs/9001`, `actions/runs/7001`; il ne voit aucun en-tête `Authorization`. La sortie ne divulgue ni les marqueurs événement, job ni corps du 404. Aucun résultat ne contient `canonical_state`; les octets du journal restent identiques après réconciliation. Le scénario passe 1/1 sous Bubblewrap `--unshare-net --unshare-pid`, avec 147 tests filtrés.

## Portée du changement et contrôles

Cette tranche ajoute un test processus ciblé au runner existant; aucun code runtime ou comportement de licence ne change. Elle couvre une même ressource répétée sous deux IDs, le cache d’erreurs amont et la projection de l’erreur par livraison, au-delà du seul chemin unitaire. `cargo test --offline --locked --all-targets --no-run` compile toutes les cibles; `cargo fmt --all -- --check`, `git diff --check`, `py_compile` du runner et du helper de manifeste, la garde Cargo, le contrôle de whitespace et les manifestes append-only passent. Un premier `py_compile` a été lancé avant la création du helper et a échoué sur son absence; ce résultat de séquencement est conservé dans `pycompile.log`, puis le rerun après création passe dans `pycompile-fixed.log`.

Un `cargo metadata --locked --offline --no-deps` courant confirme `agent-forge-core 0.1.0`, `license=null`, `license_file=null`, `publish=[]`; les métadonnées personnalisées restent `no_license_selected`, `not_assessed`, `NOASSERTION`. Aucun choix de licence n’a été fait. La recherche des implémentations locales `AgentRuntimeV1` ne trouve que `FixtureRuntime` sous `tests`; aucun adaptateur AgentOS ou modèle utilisable n’est constaté, donc aucun appel LLM n’a lieu.

## Limites

Un scénario, un job/run et deux IDs (`n=1`), réponses HTTP synthétiques sur loopback. Il ne vérifie pas un serveur Forgejo ou runner Actions réel, des ACL révoquées, un changement concurrent entre readbacks, une livraison HMAC venant de Forgejo réel, une intégration AgentOS/LLM ou un gain comparatif. Les tests d’interface ne transforment pas le statut Actions en preuve indépendante de CI. Le projet reste incomplet.
