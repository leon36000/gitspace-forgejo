# WEBHOOK-CONCURRENT-CONFLICTING-DELIVERY-001 — résultat local

## Question vérifiée

Quand deux requêtes HTTP signées simultanées réutilisent le même ID de livraison avec des corps différents, le handler n’accepte-t-il qu’un seul corps et ne conserve-t-il qu’un seul événement durable ?

## Résultat

`tests/webhook_handler_contract.rs::concurrent_changed_payloads_with_one_delivery_id_accept_once_and_conflict_once` exécute 16 courses synchronisées. À chaque course, les deux requêtes passent par le parseur HTTP, la validation HMAC, le handler webhook et le journal partagé : une réponse est `202` avec `duplicate=false`, l’autre `409` avec `delivery_conflict`. Le journal ajoute exactement un événement par ID, et l’événement gagnant contient l’un des deux corps signés.

La suite complète du contrat handler passe : 13 réussis, 0 échec, 0 ignoré, 0 filtré; le run enregistré termine en 0,20 s. `cargo fmt --all -- --check` et `git diff --check` passent également. Commandes, sorties, versions et hashes sont conservés ici; le manifeste local couvre les six artefacts.

## Portée et limites

Cette tranche ajoute une régression répétée au handler Rust; aucune logique de production n’a nécessité de correction, puisque le journal et le handler appliquaient déjà le conflit atomiquement. Le processus complet `forgejo-webhook-local` a un scénario distinct `tests/webhook_receiver_process_e2e.rs::actual_receiver_process_handles_parallel_delivery_conflict_submissions_across_restart`, qui vérifie une course concurrente réelle du binaire et les mêmes résultats après redémarrage; il n’a pas été relancé dans ce lot. Le nouveau test n’ajoute donc pas une prétention de nouveau parcours Forgejo.

Les 16 courses partagent un journal local dans un seul processus; elles ne couvrent pas plusieurs machines, plusieurs systèmes de fichiers ou une panne au milieu de l’écriture. Le test ne fait aucun appel à Forgejo, AgentOS, un LLM ou un service externe. Le garde Cargo reste sans choix de licence et sans publication.
