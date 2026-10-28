# GITLAB-PC4-CREDENTIAL-SCOPE-002 — recherche de l’accès pour le dépôt demandé

Date : 28 octobre 2026. L’utilisateur a indiqué que les accès GitLab se trouvaient dans son courriel après avoir demandé le transfert vers PC4. Recherche limitée aux mentions du projet `gitspace-forgejo` ou de l’hôte `pc4.tail0c55ff.ts.net`; trois résultats de métadonnées ont été trouvés.

## Résultats et traitement

Les résultats identifient un token GitLab pour Nexora WorkStudio/Web, un message d’accès GitLab/SonarQube général PC4, et un message intitulé mot de passe initial GitLab PC4. Le corps du seul message général PC4 a été lu pour vérifier sa portée : il mentionne un compte/mot de passe root, sans liaison à `root/gitspace-forgejo` ni scope projet. Le corps du message Nexora et celui du mot de passe initial root n’ont pas été lus. Aucune valeur de secret n’apparaît dans ce reçu, le terminal, le dépôt ou une requête GitLab; aucune authentification n’a été tentée. Le token Nexora ne convient pas à la cible. La précédente note CONT-391 reste applicable : le token Nexora découvert lors d’une recherche antérieure ne doit pas être réutilisé et devrait être renouvelé s’il ne l’a pas déjà été.

## Décision

Ne pas utiliser de compte root, mot de passe initial ou token d’un autre périmètre pour publier ce projet. Le credential requis est un Project Access Token ou Deploy Token limité au projet GitLab `root/gitspace-forgejo`, avec le minimum de permissions Git pour pousser une branche (scope `write_repository` si le type de token le permet). À défaut, demander à l’utilisateur de le créer/désigner et de le mettre à disposition via un mécanisme local de credentials, sans l’envoyer dans le dépôt ou les logs.

Le réseau reste bloqué avant authentification : PC4 répond au ping Tailscale, mais DNS et connexions TCP d’application ne passent pas depuis l’environnement Codex. Aucun `ls-remote`, staging, commit ou push n’a eu lieu. Après rétablissement de la route, lire les refs sans écriture, confirmer le projet cible et travailler sur une branche dédiée.
