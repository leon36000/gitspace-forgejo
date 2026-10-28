# Préflight GitLab PC4 — transfert du dépôt

Date : 28 octobre 2026. Vérification réseau ciblée après la demande de transfert vers `https://pc4.tail0c55ff.ts.net/root/gitspace-forgejo.git`. Worktree `/home/pc6/Documents/Codex/2026-10-21-mission-construire-une-forge-con-ue/.runtime/webhook-005-workspace`, branche `local/WEBHOOK-005-canonical-links`, HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`.

## Observations

- Le remote `pc4-gitlab` est déjà configuré vers le dépôt demandé; son URL ne contient aucun identifiant. Le remote Forgejo local `origin` reste configuré séparément.
- `tailscale status` liste PC4 et `tailscale ping -c 1 pc4` répond `pong` depuis PC6. Le peer Tailscale est donc joignable au niveau de l’overlay.
- `getent hosts pc4.tail0c55ff.ts.net` ne retourne aucune adresse. `curl https://pc4.tail0c55ff.ts.net/api/v4/version` échoue avec `Could not resolve host`.
- Après résolution explicite vers l’adresse Tailscale annoncée pour PC4, une connexion HTTP(S) vers TCP 80 et 443 expire. La sonde TCP ciblée vers 80, 443 et 22 expire également. Aucun service GitLab/SSH ne répond sur les ports documentés depuis cet environnement.
- `GIT_TERMINAL_PROMPT=0 git ls-remote https://pc4.tail0c55ff.ts.net/root/gitspace-forgejo.git` échoue à la résolution avant toute lecture de refs. La tentative n’a pas authentifié de credential.

Commandes utilisées : `tailscale status`; `tailscale ping -c 1 pc4`; `getent hosts pc4.tail0c55ff.ts.net`; `curl --connect-timeout 5 --max-time 10 -sS -D - -o /dev/null https://pc4.tail0c55ff.ts.net/api/v4/version`; `curl --resolve pc4.tail0c55ff.ts.net:443:100.76.101.48 ...`; `curl --resolve pc4.tail0c55ff.ts.net:80:100.76.101.48 ...`; une sonde Python `socket.connect` sur TCP 80/443/22; et `GIT_TERMINAL_PROMPT=0 git ls-remote` sur le remote déclaré.

## Identifiants et limites

Le courriel contient un jeton de projet pour deux autres dépôts Nexora, pas pour `root/gitspace-forgejo`; il n’est pas utilisé. Conformément au reçu antérieur de compromission, ce jeton reste exclu. Le credential administrateur root n’a pas été utilisé. Aucun secret n’a été copié dans le workspace ni transmis au serveur. Aucun fetch, lecture de refs, commit ou push GitLab n’a réussi ou été tenté après la sonde non authentifiée.

Le checkout local reste très modifié (1 783 chemins signalés par `git status --porcelain=v1` au relevé initial). Cela comprend beaucoup de rapports locaux non suivis; les éléments exportables et les secrets/caches devront être revus avant transfert. Aucun changement local n’a été écrasé. Aucune migration ou synchronisation n’est démontrée.

## Suite nécessaire

Rétablir le DNS Tailscale et le service GitLab HTTPS (ou fournir l’URL et le port réellement joignables depuis PC6). Émettre ensuite un credential neuf limité au seul dépôt `root/gitspace-forgejo` et aux écritures Git nécessaires. Après rétablissement, lire les refs distantes sans mutation, auditer visibilité/contenu de la cible, puis préparer un export contrôlé sans force-push et sans secrets, sessions locales ou caches.
