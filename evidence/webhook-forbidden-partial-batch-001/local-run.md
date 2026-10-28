# WEBHOOK-FORBIDDEN-PARTIAL-BATCH-001 — refus d’autorisation sans arrêt du batch

Date : 25 octobre 2026. `HEAD` observé : `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`; le checkout est fortement modifié. Les hashes des fichiers exécutés et des sources concernées sont dans `source-sha256.txt`.

## Parcours et résultat

Le scénario `actual_reconcile_cli_process_continues_mixed_batch_after_forbidden_resource` signe et livre trois événements `issues` au receiver réel : issue 41, issue 42, puis une nouvelle livraison pour l’issue 41. Le receiver accepte chaque événement et l’ajoute à l’inbox durable. Le CLI de réconciliation lit ensuite une fixture HTTP loopback qui renvoie `403` pour `/issues/41`, puis `200/open` pour `/issues/42`.

Le batch rend deux résultats `inconclusive/upstream_forbidden` non réessayables pour les livraisons de l’issue 41 et un résultat `current_state_read` avec état `open` pour l’issue 42. Deux GET seulement montrent que le refus est réutilisé pour le doublon et que le sibling est tout de même lu. Le code de sortie du CLI est `1`, car le lot est partiellement inconclusif. L’inbox est relue après traitement et son contenu reste identique octet pour octet. Aucun Authorization ni corps d’erreur amont n’apparaît dans les requêtes ou résultats.

## Vérification et limites

Le runner compile une copie du dépôt hors ligne avec `--locked`, puis exécute le test sous Bubblewrap avec namespaces réseau/PID séparés, home temporaire et limites `prlimit`. Le scénario ciblé passe `1/1`; 43 autres tests sont filtrés. Les commandes, résultat et versions sont joints.

Cette tranche ajoute un test processus et l’entrée correspondante du runner; la logique runtime n’a pas changé. La fixture n’est pas un serveur Forgejo réel et ne démontre pas les permissions d’un dépôt déployé. Aucun LLM, AgentOS, secret ou service externe n’est utilisé. La licence Cargo demeure non choisie; son readback courant est archivé avec le lot précédent dans [WEBHOOK-PR-DELETED-RESOURCE-PROCESS-001](../webhook-pr-deleted-resource-process-001/local-run.md). Le parcours LLM et la comparaison contrôlée restent ouverts.
