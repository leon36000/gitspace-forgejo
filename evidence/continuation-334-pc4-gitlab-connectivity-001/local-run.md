# CONTINUATION-334 — réévaluation de la connexion au GitLab PC4

Date : 28 octobre 2026. Cible demandée par l’utilisateur : `https://pc4.tail0c55ff.ts.net/root/gitspace-forgejo.git`. Le worktree reste sur `local/WEBHOOK-005-canonical-links`; aucun changement Git distant n’a été effectué.

## Résultats réseau

- `tailscale ping --c 1 --timeout 3s pc4` répond par un chemin direct; `tailscale status --json` indique PC4 en ligne.
- `getent ahostsv4 pc4.tail0c55ff.ts.net` ne résout pas le nom MagicDNS.
- À l’adresse Tailscale du pair obtenue localement, les connexions TCP 22, 80, 443, 5050, 8080 et 8443 expirent (essais parallèles, délai de 1,25 s chacun).
- `curl --resolve pc4.tail0c55ff.ts.net:443:<adresse-du-pair> --connect-timeout 3 --max-time 5 https://pc4.tail0c55ff.ts.net/users/sign_in` échoue après 3 s : `curl 28`, `http_code=000`; la connexion TCP n’est pas établie.

Le journal réseau expurgé est [network-probe.txt](network-probe.txt). L’adresse privée du pair n’est pas recopiée dans le reçu.

## Portée

Le pair Tailscale est joignable, mais le service GitLab n’accepte aucune connexion sur les ports testés depuis ce poste. Aucun `git ls-remote`, login, lecture de refs, transfert d’identifiants, création de branche distante ou push n’a été effectué. `origin` demeure le Forgejo de test local. Le courriel d’accès initial consulté antérieurement indique une rotation obligatoire au premier login; aucun mot de passe n’a été utilisé ou changé.

Le transfert demandé est bloqué avant authentification. Pour le reprendre, PC4 doit rendre HTTPS (443) ou SSH (22) joignable sur Tailscale, avec DNS MagicDNS/TLS cohérents et ACL/pare-feu autorisant ce poste. Après rétablissement, la prochaine étape sera une lecture seule des refs et de l’état du dépôt, puis une branche dédiée sans écrasement de refs existantes. Aucun secret ne doit entrer dans le dépôt ou les rapports.
