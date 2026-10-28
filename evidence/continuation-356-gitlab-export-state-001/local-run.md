# CONTINUATION-356 — état courant du transfert GitLab PC4

Date : 28 octobre 2026. Source : worktree local local/WEBHOOK-005-canonical-links, base f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0. Destination demandée : https://pc4.tail0c55ff.ts.net/root/gitspace-forgejo.git.

## État réseau et accès

Le pair PC4 répond à tailscale ping depuis le poste, mais le nom MagicDNS ne se résout pas ici. Une requête HTTPS bornée vers /users/sign_in, résolue temporairement vers l’adresse du pair, expire après 5 s sans établir TCP; HTTP/80 expire aussi. TCP/22 reste fermé. Aucun code HTTP, ls-remote ou appel GitLab authentifié n’a abouti. Aucun credential n’a été envoyé et aucune ref distante n’a été lue.

La recherche courriel fait apparaître un jeton destiné aux projets Nexora, sans portée démontrée sur root/gitspace-forgejo; il n’est pas utilisé. Le connecteur Gmail a inclus le secret dans le snippet retourné même avec la représentation metadata. Sa valeur n’est reproduite dans aucun fichier et n’a été envoyée à aucun service. Cet incident répète l’exposition signalée dans CONTINUATION-344 : révoquer/renouveler ce jeton. Ne pas utiliser le mot de passe root initial; préférer un credential à moindre privilège, scoped au projet cible et installé localement, lorsque l’endpoint est accessible.

## Instantané local et revue d’export

Le worktree reste non staged : 19 fichiers suivis modifiés et 13 857 entrées non suivies. Le checkout compte 13 951 chemins Git non ignorés représentant 176 109 497 octets (167,95 MiB), hors éléments ignorés tels que target. Le plus grand fichier candidat mesuré est inférieur à 6 MiB. Les dossiers .claude, les caches non indexés et le dépôt Git imbriqué restent exclus du transfert conformément aux audits antérieurs; aucun de ces éléments n’a été supprimé.

Un balayage heuristique des fichiers texte candidats et de 2 712 membres dans 28 archives ZIP a trouvé zéro motif de PAT GitLab/GitHub, clé AWS reconnaissable, bloc de clé privée ou URL avec userinfo. Ce résultat n’est pas une garantie d’absence de secrets; les formats binaires non décodés et les motifs inconnus restent hors couverture. Le balayage automatisé a parcouru les octets de .claude/ralph-loop.local.md sans en rendre ni stocker le contenu; conformément à la consigne antérieure sur les prompts locaux, ce fichier ne sera pas examiné ni exporté automatiquement.

La branche locale est à zéro commit devant et sept commits derrière son upstream local origin/main, actuellement 6030e13e3887926f8629cbb22f785d5b4bab580a. origin pointe toujours vers Forgejo sur loopback, pas GitLab. Le dépôt GitLab demandé n’a pas été inspecté; son état reste inconnu. Aucun staging, commit, push ou modification de configuration n’a été effectué.

## Suite

Quand TCP/443 sera joignable depuis ce pair Tailscale et qu’un credential scoped au projet sera configuré dans le gestionnaire local, lire les refs distantes avant toute préparation, conserver les changements locaux, préparer une branche dédiée sans force-push et relire le diff complet avant l’envoi. Continuer les lots locaux pendant le blocage. Les vérifications précédentes d’isolation et du refus JSON des erreurs de pipe credential existent déjà (CONTINUATION-307 et CONTINUATION-269); elles n’ont pas été rejouées.
