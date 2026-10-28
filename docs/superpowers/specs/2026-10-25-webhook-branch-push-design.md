# Réconcilier les notifications Forgejo `push` vers la tête actuelle d’une branche

## Intention

Permettre au récepteur et au CLI local de transformer un webhook `push` de Forgejo en lecture fraîche de la branche concernée. Les SHA du payload servent à valider la forme et à situer la notification; ils ne deviennent jamais la tête canonique. Cela permet notamment de constater une avancée de branche de base qui n’est pas représentée par une notification PR.

## Choix et options

1. **Ignorer `push` et attendre le webhook `pull_request` avec `action=synchronized` de Forgejo v17.** Aucun changement de code, mais une mise à jour d’une branche de base peut ne produire aucun état canonique consommable pour invalider une preuve.
2. **Faire confiance à `payload.after`.** Peu coûteux, mais une notification retardée peut réintroduire une tête périmée; le payload ne remplace pas une lecture courante.
3. **Ajouter une ressource branche et appeler `GET /repos/{owner}/{repo}/branches/{branch}`.** Le contrôleur HTTP ne fait qu’enregistrer le webhook signé; le CLI revalide le dépôt, le ref et la réponse, puis retourne le SHA et l’horodatage du commit courant. C’est l’option retenue pour cette tranche.

## Portée

- Ajouter `push` aux événements pris en charge, seulement pour `refs/heads/<branch>` valides.
- Ajouter une identité de ressource typée pour un numéro issue/PR ou un nom de branche.
- Rejeter les tags, refs mal formés, dépôts hors portée, nom de branche et SHA de commit API invalides.
- Le résultat branche utilise `state="present"`, l’URL humaine dérivée de l’origine allowlistée, l’horodatage du commit et sa tête SHA actuelle.
- Une réponse API 404 reste `inconclusive/not_found`; le client ne peut pas distinguer une branche supprimée d’une ressource masquée/inaccessible avec cette seule route. Aucun résultat `deleted` n’est inventé.
- Les événements issues/PR et leurs champs de sortie existants restent identiques.

## Réutilisation et sécurité

Réutiliser `ForgejoCurrentState`, la politique de délai/reprise, la liste fixe des profils loopback et le cache par ressource. Les noms de branche sont validés selon les règles de ref Git et encodés comme un segment d’URL. Aucun token, origine, owner ou dépôt fourni par le webhook ne peut élargir la portée configurée. L’adaptateur reste une fixture locale, pas un connecteur multi-tenant ni une intégration AgentOS.

## Vérification attendue

Un test contrat doit montrer qu’une notification `push` ancienne produit la tête renvoyée par l’API courante, pas son champ `after`; un 404 reste inconclusif et ne crée pas d’état. Un test processus doit passer par le receiver HMAC réel, l’inbox durable, le CLI réel et un serveur HTTP loopback, vérifier l’URL branche encodée, l’absence d’Authorization et l’inbox inchangée. Les événements existants seront ciblés après l’ajout; aucun Forgejo vivant, LLM, coût ou service externe n’est requis.
