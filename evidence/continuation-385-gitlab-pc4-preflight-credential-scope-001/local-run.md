# CONTINUATION-385 — préflight GitLab PC4 et périmètre des accès

Date : 28 octobre 2026. Worktree local `webhook-005-workspace`, branche `local/WEBHOOK-005-canonical-links`, HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`.

## Joignabilité depuis PC6

`tailscale status --json` indique l’état local `Running` et le pair `pc4` en ligne; `tailscale ping --c 1 pc4` réussit. Le résolveur système ne trouve pas `pc4.tail0c55ff.ts.net`. Une requête HTTPS anonyme à `/users/sign_in` échoue avant HTTP avec `Could not resolve host`. Une seconde tentative sans identifiants a forcé le nom TLS sur l’adresse Tailscale courante du pair; TCP/443 expire en cinq secondes avant TLS. Le port TCP 22, 80 et 443 est injoignable depuis PC6.

Un `git ls-remote` anonyme, sans prompt interactif ni credential helper, n’a pas rendu de refs pendant l’attente et le processus lancé pour ce contrôle a été arrêté. Aucune authentification GitLab ni lecture de refs n’a réussi.

## Revue des accès indiqués par l’utilisateur

La recherche de courriels limitée aux messages concernant le nom PC4 et le dépôt GitLab a identifié un PAT destiné à deux dépôts Nexora, ainsi que des identifiants initiaux de compte administrateur. Le PAT ne couvre pas `root/gitspace-forgejo`; les identifiants administrateur ne sont pas utilisés afin d’éviter un privilège plus large que nécessaire. Aucun corps de message, identifiant de credential ou secret n’est copié dans le dépôt ni dans les journaux.

## Résultat

Le remote `pc4-gitlab` est configuré sur `https://pc4.tail0c55ff.ts.net/root/gitspace-forgejo.git`; `origin` reste le Forgejo local. Les deux remotes n’ont pas été modifiés. Aucun `fetch`, staging, commit, push ou transfert de fichier n’a eu lieu. Au relevé avant création de ce reçu, le worktree comptait 19 entrées suivies en état modifié et 14 422 fichiers non suivis; ce relevé ne constitue pas encore une sélection d’export et certains caches générés doivent être exclus.

Prochaine action : rendre le GitLab PC4 joignable depuis PC6 sur HTTPS via Tailscale/MagicDNS, puis fournir un credential `write_repository` limité au projet `root/gitspace-forgejo`. À la reprise, lire les refs distantes avant toute écriture, auditer le contenu exportable puis pousser une branche dédiée sans toucher à `main`.

Limites : contrôle ponctuel depuis PC6; il ne prouve pas si GitLab fonctionne localement sur PC4, ni l’état d’authentification ou les protections du projet. Le transfert demandé et la continuation sur GitLab restent non réalisés.
