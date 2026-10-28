# FORGEJO-NATIVE-DELETED-RESOURCE-006 — livraison native avant suppression

> Exécution préliminaire. Le wrapper de cette première passe n’a conservé que le corps vide et le succès HTTP, pas le code de statut exact. La répétition ciblée instrumentée et la preuve de référence sont dans [FORGEJO-NATIVE-DELETED-RESOURCE-007](../forgejo-native-deleted-resource-007/local-run.md).

Date : 25 octobre 2026. Code produit observé au commit `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`, branche `local/WEBHOOK-005-canonical-links`, avec worktree fortement modifié. Les empreintes du runner et des sources liées sont conservées dans `source-sha256.txt` et `provenance.json`.

## Résultat exécuté

Le runner en mode `--native-deleted-resource-only` a lancé une instance Forgejo v17.0.0 jetable sur loopback et deux receivers/adaptateurs locaux. Forgejo a créé les issues #2 et #3 avec un webhook `issues/opened` actif. Chaque livraison a retourné 202 au receiver; les tâches Forgejo sont reliées aux UUID reçus et aux deux séquences du journal append-only. La vérification d’intégrité relie aussi les corps HTTP aux empreintes de tâches. L’issue #2 est l’issue sœur, maintenue existante.

Après la livraison native de #3, l’identité propriétaire supprime cette issue par l’API et lit le même numéro : `DELETE` retourne 204 (réponse vide), puis `GET` propriétaire retourne 404. Le réconciliateur reçoit les deux UUID, dans l’ordre #3 puis #2. L’observateur HTTP indépendant confirme les lectures authentifiées de l’identité non-admin read-only : `GET issues/3 -> 404`, puis `GET issues/2 -> 200`. Le CLI sort avec 1 et retourne un verdict `inconclusive`; #3 porte `resource_not_found`, `retryable=false`, sans état canonique; #2 conserve son état canonique `open`. Le credential de réconciliation est consommé, le journal reste byte-for-byte identique et le readback propriétaire disambiguïse l’absence dans ce bac à sable.

Les états complets, tâches et observations sont dans `result.json`, `hook-tasks.json`, `webhook-events.json`, `api-observation-proxy.json` et `http-proxy.json`. Le manifeste local couvre les artefacts du dossier.

## Références et vérifications

Le source officiel Forgejo v17.0.0 a été examiné dans le scratch `/tmp/forgejo-v17-base-cas-poc`, `HEAD=0d791421f85ca13a63b665053d81ea765fbbc571`. Les trois fichiers cités étaient propres par rapport à ce commit lors de la lecture : `routers/api/v1/api.go:1173-1176` enregistre le DELETE avec `reqToken()` et `reqAdmin()`; `routers/api/v1/repo/issue.go::DeleteIssue` à `:962-1000` charge l’issue, appelle le service puis retourne 204; `services/issue/issue.go::DeleteIssue` à `:171-201` supprime les entrées et déclenche la notification. L’audit de ces symboles ne prétend pas inventorier toutes les routes d’écriture Forgejo.

Le binaire local rapporte `17.0.0+gitea-1.22.0`, SHA-256 `e5ec437cd6357e8d1b4580cb47ab1daa58a32b33971a63db63446a535d33adb7`; son commit exact de build n’est pas établi. Le runner a compilé les deux binaires Rust nécessaires `--offline --locked` dans Bubblewrap 0.9.0, avec namespaces réseau/PID, environnement vidé et sources/sysroot staged en lecture seule. Rust/Cargo sont en version 1.95.0. La suite ciblée `tests/test_forgejo_webhook_native_replay_e2e.py` passe 27/27, `py_compile` passe et les deux fichiers Python n’ont aucun espace final. La suite Python complète et les tests Rust complets n’ont pas été relancés.

Le readback Cargo conservé dans `cargo-license-readback.json` confirme `license=null`, `license_file=null`, `publish=[]`, `decision=no_license_selected`, `redistribution_rights=not_assessed`. Aucun choix de licence n’a été fait. L’état privé de Forgejo reste sous `/tmp/forgejo-native-deleted-resource-006-state`, hors preuves; le rapport ne contient aucun token. Le nettoyage de la preuve indique Forgejo, receiver et proxies arrêtés, sans listener résiduel et sans échec de nettoyage.

## Limites et prochaine action

Ce run prouve qu’une livraison Forgejo native déjà acceptée peut devenir obsolète après suppression et qu’un 404 pour une ressource ne masque pas la lecture réussie d’un frère distinct. Un 404 côté lecteur demeure normalement ambigu entre absence et perte d’accès; le readback propriétaire sert seulement à établir la cause dans cette fixture. Un seul dépôt et une seule instance sous l’identité hôte ont été utilisés; aucune concurrence, code hostile, modèle LLM, AgentOS, coût de modèle, PR/fusion ou comparaison de valeur n’est testé ici. La livraison est un événement natif de création; aucun événement webhook de suppression n’est supposé.

Suite locale utile selon le checkpoint : reproduire l’annulation du superviseur pendant `cargo.test` en vol et observer le groupe du vérificateur et ses descendants. Le projet reste incomplet.
