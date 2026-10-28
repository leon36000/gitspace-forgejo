# GITLAB-PC4-TRANSFER-001 — prévol de transfert et connectivité

Date : 28 octobre 2026. Worktree examiné : `local/WEBHOOK-005-canonical-links`, HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. Destination demandée par l’utilisateur : dépôt self-managed GitLab PC4 `root/gitspace-forgejo`. Aucun contenu n’a été écrit sur le serveur.

## Prévol du contenu

Le remote `pc4-gitlab` existe déjà dans la configuration du dépôt. Le worktree compte 14 946 fichiers non suivis non ignorés (~168 291 201 octets), 19 fichiers suivis modifiés au moment du relevé, ainsi qu’un `.claude/` local non lu. Le dossier `.runtime/` occupe environ 13 GB et est ignoré par la règle Git `.runtime/`; un balayage de motifs a trouvé six fichiers privés sous des données locales runtime, sans en lire le contenu. Ces données et `.claude/` ne doivent pas être transférés. Le balayage des chemins exportables n’a pas trouvé de chaîne correspondant aux formes courantes `glpat-`, GitHub PAT ou `Bearer`, mais n’est pas une revue exhaustive de tous les secrets et données privées.

Le transfert éventuel doit porter sur les sources, tests, docs et preuves de ce projet, après revue et vérification de la visibilité/cible. Le worktree est fortement modifié et le dépôt n’a pas été relu fichier par fichier pour publication. Il ne faut pas `git add` globalement les données runtime ni l’état Claude local. Toute écriture distante doit utiliser une branche dédiée, sans pousser `main`, sans force-push et sans embarquer de credentials.

## Résultat réseau

- Le DNS système ne résout pas `pc4.tail0c55ff.ts.net` (`getent`/`curl`: name resolution failure).
- Tailscale CLI 1.102.4 signale PC4 en ligne; `tailscale ping --timeout=4s pc4` reçoit une réponse au niveau Tailscale. Le namespace d’exécution ne présente pas `tailscale0` ni une route système vers le réseau Tailscale.
- Les essais HTTPS avec le nom et avec une résolution locale temporaire échouent avant TLS; TCP 22, 80 et 443 sont injoignables depuis cet environnement. Les ports 22/80/443 ont été testés seulement sur l’hôte PC4 fourni par l’utilisateur.
- `git config` ne signale aucun credential helper. Aucun courriel n’a été recherché ou lu et aucun secret n’a été utilisé.
- Aucun `git ls-remote` n’a abouti; aucun staging, commit, branche distante, push, suppression ni modification du serveur n’a eu lieu.

## Conclusion et suite

Le blocage est réseau, avant authentification. Une réponse Tailscale du daemon ne prouve pas que l’espace d’exécution puisse ouvrir des connexions applicatives. Il faut rendre le TCP 443 GitLab joignable depuis l’environnement Codex (DNS/route Tailscale/TUN), ou déplacer le checkout vers un environnement qui a cette connectivité. Ensuite seulement : lecture anonyme/authentifiée des refs, confirmation de la destination, sélection d’un credential de périmètre projet minimal et création d’une branche de transfert. En attendant, poursuivre les tests et modifications locales; ne pas prétendre que le dépôt a été migré.
