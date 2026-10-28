# MISSION-012-COMPOSE-063 — échec actionnable du lien SHA de fusion

Date : 25 octobre 2026. Exécution locale de la composition avec `local-test-adapter`, sans appel de modèle. Le dossier conserve les sorties partielles utiles; l’échec n’est pas compté comme un parcours complet.

## Résultat

L’admission a accepté le webhook Forgejo de l’issue et le fixture a corrigé `src/lib.rs` après l’échec d’acceptation initial. La phase PR a ensuite retourné `FAIL-CLOSED: 'merge_commit_sha'`. Le reçu de l’autorité locale dans [rust-merge-authority-result.json](rust-merge-authority-result.json) confirme que le merge s’était déjà produit dans l’instance jetable et que la tête `main` était `12e4f35993262a9abf101bbe1af841a496bd5ed7`. Le runner s’est donc arrêté après l’effet de merge, avant de produire une preuve complète de réconciliation.

La cause est reproductible dans le code : `parse_live_pull_request_delivery_record` ne renvoie `merge_commit_sha` que lorsque `expected_merged=True`, mais l’observateur de PR n’envoyait pas ce paramètre pour la notification `closed`. L’accès obligatoire au champ provoquait la `KeyError`. Le constat expurgé est dans [first-failure-observation.json](first-failure-observation.json). La tentative n’a pas été rejouée sur le même état muté; le run 064 est parti d’un nouvel état privé.

## Reprise vérifiée

Le parseur reçoit maintenant `expected_merged=True` pour l’événement `pull_request/closed`. Le composeur versionne le reçu comme schéma 5 et refuse un SHA absent ou différent dans le webhook fermé, l’état canonique ou l’un des trois reçus. Le test unitaire couvre ces refus; le parcours 064 valide ensuite le comportement avec un webhook natif Forgejo réel et une réconciliation en boucle locale.

## Limites

Le run 063 n’est pas un succès bout en bout et n’atteste pas que les trois événements ont été réconciliés. Le merge de test a eu lieu avant l’échec du harnais; l’état privé et les secrets du bac à sable n’ont pas été copiés dans ce dossier. Pas de LLM, AgentOS, RAG, coût de modèle ou comparaison conventionnelle. Les fichiers de cette preuve partielle sont inscrits dans le manifeste local.
