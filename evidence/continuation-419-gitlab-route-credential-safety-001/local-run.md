# CONTINUATION-419 — route GitLab PC4 bloquée; identifiants exposés dans un extrait

Date : 28 octobre 2026 (America/Montreal). Worktree inspecté : `/home/pc6/Documents/Codex/2026-10-21-mission-construire-une-forge-con-ue/.runtime/webhook-005-workspace`, branche `local/WEBHOOK-005-canonical-links`, HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. Dépôt cible demandé : `https://pc4.tail0c55ff.ts.net/root/gitspace-forgejo.git`.

## Recherche d’accès et incident de confidentialité

À la demande de l’utilisateur, les recherches Gmail ciblées par nom d’hôte PC4, chemin de dépôt et « GitLab self-managed PC4 » ont retourné respectivement 3, 0 et 2 candidats. La lecture `format=metadata` demandée pour cinq candidats a tout de même renvoyé des extraits automatiques. Ceux-ci contenaient un PAT GitLab destiné à d’autres projets et le mot de passe initial du compte `root`. Leurs valeurs ne sont pas recopiées dans ce reçu, aucun fichier, commande, manifeste ou appel distant. Le PAT n’a pas été utilisé. Aucun login/API GitLab, lecture de corps ou pièce jointe, ni mutation Gmail n’a eu lieu.

L’utilisateur a été averti immédiatement : révoquer le PAT, faire tourner le mot de passe initial et ne pas renvoyer de secret dans le courriel. Le compte/token trouvé ne constitue pas une autorisation limitée à `root/gitspace-forgejo`. La recherche Gmail est désormais fermée pour cette mission. Les journaux internes de l’appel metadata ont reçu des valeurs de secret; il faut traiter ces identifiants comme compromis.

## Connectivité PC4 sans authentification

- `tailscale status --json` : backend `Running`, pair PC4 `online`.
- `getent hosts pc4.tail0c55ff.ts.net` : aucune adresse résolue.
- L’IP du pair lue localement dans l’état Tailscale a été utilisée sans credential avec `curl --head --silent --show-error --connect-timeout 3 --max-time 7 --resolve pc4.tail0c55ff.ts.net:443:<IP-du-pair> https://pc4.tail0c55ff.ts.net/ -o /dev/null -w ...` : TCP/443 expire après environ 3,0 s; aucun HTTP ni TLS n’est établi. L’IP privée elle-même n’est pas conservée dans les artefacts.
- Aucun `git ls-remote`, clone, API authentifiée, fetch, staging, commit ou push.

## État local et suite

`git status --short` compte 1 830 chemins suivis modifiés ou non suivis; branche et HEAD restent comme ci-dessus. Rien n’a été staged, commit, nettoyé, remplacé ou supprimé. Seuls ce reçu, son index de preuve, le checkpoint et leurs entrées d’index append-only sont ajoutés.

Le transfert reste bloqué sur deux prérequis indépendants : rétablir DNS/TCP 443 depuis PC6 et configurer localement un credential neuf à moindre privilège, limité au projet cible. Après leur résolution, lire les refs distantes, répéter l’audit des chemins exportables et pousser uniquement une branche `codex/` sans force-push. Aucun changement produit ou test logiciel n’est inclus dans ce lot; aucun résultat de transfert n’est revendiqué.
