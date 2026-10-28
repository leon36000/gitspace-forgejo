# FORGEJO-NATIVE-DELETED-RESOURCE-007 — événement natif rendu obsolète par suppression

Date : 25 octobre 2026. Code produit observé au commit `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`, branche `local/WEBHOOK-005-canonical-links`; l’arbre de travail reste fortement modifié. Les empreintes du runner, du wrapper API et des tests sont dans `source-sha256.txt` et `provenance.json`.

## Résultat local

Le runner ciblé `--native-deleted-resource-only` démarre Forgejo v17.0.0 sur loopback. Il laisse d’abord la livraison de l’issue #1 échouer sur le secret de receiver volontairement invalide (401), puis redémarre le receiver avec le secret local valide. Les issues #2 et #3 sont créées via l’API; pour chacune, une tâche Forgejo `issues`, une livraison HTTP 202 et un événement du journal sont liés par l’UUID, la séquence et l’empreinte du corps HTTP.

Après réception native de `issues/opened` pour #3, l’identité propriétaire exécute `DELETE /repos/forge-admin/agent-forge-core/issues/3`. Le wrapper conserve le statut HTTP : réponse 204 vide. Le readback propriétaire du même numéro retourne 404. Le réconciliateur traite ensuite les UUID dans l’ordre #3, #2 avec l’identité non-admin read-only. Le proxy HTTP indépendant observe exactement `GET issues/3 -> 404`, puis `GET issues/2 -> 200`, tous deux authentifiés avec ce lecteur. Le CLI sort avec 1 et rend `inconclusive`: #3 reçoit `resource_not_found`, `retryable=false`, sans état canonique; #2 conserve son état canonique `open`. Le credential privé est consommé, le journal reste byte-for-byte stable pendant la réconciliation, et les payloads des deux livraisons correspondent aux empreintes Forgejo.

Le verdict `passed` du harness signifie que ces refus et lectures correspondent aux assertions attendues; il ne signifie pas que l’issue supprimée a été lue avec succès. Les artefacts bruts filtrés sont `result.json`, `hook-tasks.json`, `webhook-events.json`, `api-observation-proxy.json` et `http-proxy.json`.

## Source Forgejo et vérifications

Le code officiel Forgejo v17.0.0 est examiné dans le scratch `/tmp/forgejo-v17-base-cas-poc` au commit `0d791421f85ca13a63b665053d81ea765fbbc571`; les fichiers ci-dessous n’avaient pas de diff par rapport à ce commit lors de l’examen. `routers/api/v1/api.go:1173-1176` rattache la route DELETE à `reqToken()` et `reqAdmin()`. `routers/api/v1/repo/issue.go::DeleteIssue` (`:962-1000`) appelle le service puis renvoie `204 No Content`. `services/issue/issue.go::DeleteIssue` (`:171-201`) supprime les entrées de l’issue et déclenche la notification. Ces symboles ne constituent pas un inventaire exhaustif des writers Forgejo.

Le binaire local indique `17.0.0+gitea-1.22.0`, SHA-256 `e5ec437cd6357e8d1b4580cb47ab1daa58a32b33971a63db63446a535d33adb7`; son commit exact de build n’est pas établi. Le runner construit le receiver et le reconciler Rust en mode `--offline --locked` sous Bubblewrap 0.9.0, namespaces réseau/PID, environnement vidé et sources/sysroot staged en lecture seule. Rust/Cargo : 1.95.0. Le module ciblé passe 27/27; `py_compile` et l’inspection des espaces finaux passent. La suite Python complète et la suite Rust complète ne sont pas relancées.

`cargo-license-readback.json` confirme `license=null`, `license_file=null`, `publish=[]`, `decision=no_license_selected` et `redistribution_rights=not_assessed`. Aucun choix de licence n’a été fait. Les processus Forgejo, receiver et proxies sont arrêtés, listeners vérifiés absents, `cleanup_failure=null`. L’état local sous `/tmp/forgejo-native-deleted-resource-007-state` n’est pas inclus dans les preuves et aucun appel LLM/service externe n’a été fait.

## Limites et suite

L’expérience teste une livraison native de création suivie de la suppression réelle avant réconciliation; elle ne présume pas qu’un événement de suppression soit envoyé. Le 404 du lecteur reste ambigu en général entre une ressource absente et une permission perdue; seul le readback propriétaire de cette fixture établit ici la suppression. Un dépôt, une instance et une exécution séquentielle sont utilisés; pas de concurrence, code hostile, parcours LLM, PR/fusion ou comparaison de valeur. Projet incomplet.

La prochaine action locale du checkpoint reste l’observation de l’annulation du superviseur pendant un `cargo.test` en vol, avec vérification des processus descendants et réparation seulement si une fuite est reproduite.
