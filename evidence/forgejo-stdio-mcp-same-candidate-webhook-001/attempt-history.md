# Historique expurgé des tentatives d’intégration

Les répertoires de bac à sable restent dans `/tmp`; cette note ne contient ni DB Forgejo, ni token, ni payload complet.

| Tentative | Observation | Suite |
|---|---|---|
| 004 | Le démarrage a échoué car la compilation isolée n’incluait pas le binaire receiver demandé. | Corrigé le helper Cargo pour compiler tous les targets demandés. |
| 005 | Le receiver démarrait, mais le runner appelait un module d’attente sous un mauvais chemin d’import. | Corrigé l’import vers le module réel. |
| 006 | La PR avait fusionné et l’acceptation post-merge passait, mais les événements Forgejo `pull_request` `opened`, `synchronized` et `closed` avec `review` JSON `null` échouaient tous à la livraison; l’événement de revue dédié avec `review` objet réussissait. | Deux tests test-first ont localisé la divergence Forgejo v17 : le champ est un pointeur nullable sérialisé sans `omitempty`. Ils exigent le refus des valeurs non nulles tout en acceptant ce pointeur nul. |
| 007 | Même source MCP écrite, PR fusionnée, événements natifs livrés, événement `closed` accepté et réconcilié vers le commit de merge exact. | Run vert décrit dans `local-run.md`. |

La correspondance des événements 006/007 est conservée uniquement sous forme de métadonnées extraites en lecture seule : `event_type`, `action`, type JSON de `review`, `delivered` et `succeeded`. Aucune valeur brute de `payload_content` ni `response_content` n’a été archivée.
