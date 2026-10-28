# CONTINUATION-378 — retest HTTPS PC4 via l’adresse Tailscale directe

Date : 28 octobre 2026. Worktree : `local/WEBHOOK-005-canonical-links`, HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`.

## Résultat

Après l’échec DNS de CONTINUATION-377, `tailscale status --json` a indiqué que le pair PC4 était en ligne et a fourni ses adresses Tailscale. Une seconde requête HTTPS anonyme a contourné uniquement la résolution DNS au moyen de `curl --resolve`, en conservant le nom d’hôte/SNI `pc4.tail0c55ff.ts.net`. La connexion TCP à 443 a échoué (`curl` code 7, HTTP `000`) sur les adresses IPv4 et IPv6 observées. La sortie filtrée est dans `connectivity-readback.txt`.

Cette observation localise le blocage avant HTTP/TLS : elle ne permet de conclure ni sur l’état du service GitLab, ni sur les credentials ou les permissions du dépôt. Aucun header d’autorisation n’a été envoyé; aucun login, appel API, `git ls-remote`, lecture de refs, fetch, commit ou push n’a eu lieu.

La recherche de courriel précédente n’a trouvé qu’un jeton pour d’autres projets; les reçus antérieurs le signalent à renouveler. Le mot de passe initial `root` a une portée administrateur et n’a pas été utilisé. Aucun secret n’est reproduit dans cette preuve.

## Suite nécessaire pour le transfert demandé

Rétablir depuis PC6 le routage tailnet vers le port HTTPS du GitLab PC4 (ou fournir un endpoint de test joignable sur ce réseau), puis émettre un credential frais avec accès lecture/écriture limité à `root/gitspace-forgejo`. Une fois le serveur joignable, la prochaine opération distante restera la lecture de l’identité et des références; la préparation d’une branche de transfert attend aussi la réconciliation de l’audit d’export CONT-336. Aucun code n’a été publié.

Les travaux locaux restent indépendants et continuent dans le worktree existant. Les preuves de connectivité ne modifient pas l’état du projet ni ses fichiers source.
