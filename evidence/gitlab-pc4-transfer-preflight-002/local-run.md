# CONTINUATION-383 — prévol GitLab PC4 depuis PC6

Date : 28 octobre 2026. Worktree local `webhook-005-workspace`, branche `local/WEBHOOK-005-canonical-links`, HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`.

## Contrôles en lecture seule

Le remote `pc4-gitlab` pointe vers `https://pc4.tail0c55ff.ts.net/root/gitspace-forgejo.git`; `origin` continue de pointer vers le Forgejo local. `tailscale status --json` indique un backend local `Running` et une entrée de pair PC4. Le FQDN ne se résout pas avec le résolveur système. `git ls-remote` a été lancé sans prompt interactif, sans configuration globale et avec le credential helper désactivé; il échoue par `Could not resolve host` avant toute lecture de refs.

Pour distinguer DNS d’un port HTTPS fermé, une seule requête `curl --head` a dirigé le nom TLS vers l’adresse Tailscale courante de PC4, sans identifiant. La connexion TCP expire après cinq secondes (`HTTP 000`), avant TLS. L’adresse privée du pair n’est pas conservée dans les artefacts.

## Résultat et portée

Le service GitLab ne peut pas être atteint depuis PC6 au moment de cette reprise. Aucune authentification, lecture de refs, modification distante, staging, commit, push ou changement du remote n’a eu lieu. Aucun secret n’est présent dans les commandes ou les journaux conservés. Le worktree local reste la seule copie de travail active.

Prochaine action distante : rendre l’endpoint HTTPS PC4 joignable via le réseau Tailscale (nom MagicDNS résolu et port 443 accessible, ou endpoint Tailscale équivalent). Après le rétablissement, vérifier les refs en lecture seule, puis auditer la destination et les fichiers modifiés/non suivis avant de préparer une branche dédiée. Les identifiants ne doivent pas apparaître dans les artefacts; ne pas pousser sur `main`.

Limites : contrôle depuis une seule machine cliente; il ne vérifie ni le service local sur PC4, ni son état d’authentification, ni les protections/projets GitLab. Aucun octet de dépôt n’a été transféré.

## Sorties conservées

- `remotes.txt` : noms et URL des remotes, sans credential.
- `gitlab-ls-remote.log` : échec DNS avant authentification.
- `tailscale-summary.txt` : état local résumé, adresse du pair omise.
- `https-forced-resolution.log` : expiration TCP sans réponse HTTP.
- `verification-commands.txt` : commandes et portée.
