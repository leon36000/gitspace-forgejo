# WEBHOOK-PR-DELETED-COLLISION-BATCH-001 — cache séparé par type de ressource

Date : 25 octobre 2026. `HEAD` observé : `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`; le checkout est fortement modifié. Les hashes de la tranche, des deux files de code et des sources de réconciliation sont dans `source-sha256.txt`.

## Parcours et résultat

Le scénario `actual_reconcile_cli_process_keeps_deleted_pull_request_collision_separate_in_batch` signe puis envoie au receiver trois événements en ordre : PR 41, issue 41, puis une seconde livraison pour la PR 41. Le receiver les accepte tous en `202` et les ajoute à l’inbox durable. Le CLI demande ensuite la PR puis l’issue au serveur HTTP loopback.

La fixture renvoie `404` pour `/pulls/41` et `200/open` pour `/issues/41`. Le résultat batch contient deux refus terminaux `resource_not_found` typés `pull_request` (aucun `canonical_state`) et une lecture canonique `issue/open`. Le fixture observe exactement deux GET : le 404 de la PR est réutilisé pour sa deuxième livraison, tandis que l’issue de même numéro reçoit son propre appel et son propre résultat. Le code de sortie du CLI est `1` pour le batch partiellement inconclusif. Aucun Authorization ou corps d’erreur amont n’apparaît, et la relecture prouve que l’inbox n’a pas changé octet pour octet.

Cette assertion intégrée vérifie à la fois le cache négatif sur les doublons et la séparation de `ResourceRef` par kind. Les cas existants couvraient ces deux propriétés séparément : même numéro avec deux ressources valides, et 404 en batch d’issues. Le nouveau scénario les compose au niveau des processus receiver et CLI.

## Vérification et limites

La commande ciblée passe `1/1`, avec 42 autres tests filtrés. Le runner compile la copie verrouillée hors ligne puis exécute le binaire sous Bubblewrap avec réseau et PID isolés, home temporaire et limites `prlimit`; seule la fixture loopback est disponible. Les commandes et versions sont dans `commands.txt` et `tool-versions.txt`.

Ce lot ajoute un test Rust processus et son entrée dans le runner. Il n’a pas modifié le code d’exécution : le chemin existant satisfait le nouveau contrat sous fixture. Ce résultat ne prouve pas le comportement de toutes les versions Forgejo, un serveur Forgejo réel, un appel LLM ou une amélioration générale. Aucun coût nouveau, secret ou service externe n’intervient. La licence Cargo reste non choisie; le readback vérifié de ses métadonnées est conservé dans [le rapport Cargo adjacent](../webhook-pr-deleted-resource-process-001/local-run.md).
