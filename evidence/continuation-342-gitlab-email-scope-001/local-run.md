# CONTINUATION-342 — portée de l’accès GitLab reçu par courriel

Date : 28 octobre 2026. Dépôt local : worktree `local/WEBHOOK-005-canonical-links`, HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. Cible fournie : `https://pc4.tail0c55ff.ts.net/root/gitspace-forgejo.git`.

## Constat sur l’accès

À la demande de l’utilisateur, une recherche Gmail limitée au nom du dépôt et au FQDN a retrouvé le courriel du 27 octobre intitulé « Jeton GitLab pour l’agent Nexora WorkStudio et Web ». Le message décrit un jeton pour Nexora WorkStudio et Nexora Web, avec le rôle Développeur sur ces deux projets. Il ne mentionne pas `root/gitspace-forgejo`. Le jeton n’a pas été utilisé, copié, enregistré ou transmis.

Le reçu CONTINUATION-334 indique qu’un accès initial séparé imposerait un changement de mot de passe interactif; aucun login ni changement n’a été entrepris.

## Connectivité recontrôlée

- `tailscale status --json` indique le pair PC4 en ligne; `tailscale ping` reçoit une réponse.
- La résolution DNS locale de `pc4.tail0c55ff.ts.net` ne retourne aucune adresse.
- La connexion HTTPS/TCP au port 443, en forçant la résolution vers l’adresse Tailscale du pair, expire sans réponse. Le port 8443 testé pour corroborer l’autre URL de service expire également.
- Le reçu CONTINUATION-334 conserve les résultats complets des sondes réseau antérieures.

## Résultat et suite

Aucune authentification GitLab, lecture de refs, création de branche ou écriture distante n’a eu lieu. Le dépôt local et ses changements préexistants restent préservés; ce lot ajoute uniquement de la documentation et des index de preuve.

Le transfert attend que GitLab soit joignable depuis PC6 sur HTTPS via Tailscale et qu’un compte ou jeton à moindre privilège ait un rôle d’écriture sur `root/gitspace-forgejo`. À la reprise, lire les refs distantes avant de créer un commit ou pousser une branche dédiée. Ne pas utiliser le jeton prévu pour les projets Nexora sur le dépôt cible.

Ce diagnostic ne prouve pas la migration GitLab, une fusion distante, une modification de production ni un parcours LLM.