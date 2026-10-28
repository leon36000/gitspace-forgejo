# GITLAB-PC4-TRANSFER-003 — synchronisation du delta local

Date : 28 octobre 2026 (America/Montreal).

## Cible et portée

- Projet GitLab PC4 privé : `root/gitspace-forgejo`, ID 4.
- Branche non-default de travail : `codex/pc6-transfer-work-2026-10-28`, autorisée en écriture (`can_push=true`). La branche par défaut protégée `codex/pc6-transfer-2026-10-28` n’a pas été modifiée.
- Commit de départ de la branche : `1e80ce761466be215c16b5a829b5a3d7c29719a6`.
- Commit de synchronisation : `cfc7590beeafc54677fc32690c176cf0629fdfac`.

## Transfert et readback

La comparaison du tree distant au worktree sélectionne les fichiers sources et documents touchés depuis le snapshot précédent, plus les reçus des lots CONTINUATION-432 et CONTINUATION-433. La synchronisation a créé 54 blobs et mis à jour 9 blobs, soit 63 fichiers texte UTF-8 et 5 386 209 octets. Les fichiers locaux ont été vérifiés contre le tree après commit : les 63 identifiants de blob Git et les modes correspondent exactement (63/63, aucune absence ni divergence). Le tree contient alors 341 blobs au total.

Le scan local ciblé des 63 fichiers n’a trouvé aucune correspondance pour les motifs connus de PAT GitLab/GitHub, token de runner, clés OpenAI/AWS, clé privée PEM, Bearer long ou identifiant/mot de passe dans une URL. Ce scan n’établit pas l’absence de tout format de secret.

Le commit a déclenché le pipeline 164 (`source=push`) sur ce SHA. Son état était `running` au relevé `2026-10-28 17:46:43 -04:00`; aucun résultat final n’est revendiqué ici. Le pipeline 163 du commit précédent était `success`, mais il ne valide pas ce commit.

## Limites et suite

Le dépôt PC4 est une copie de travail par commits API, pas un transfert des refs ni de l’historique local. Les 28 ZIP de snapshots binaires, caches et sorties ignorées, `.runtime`, `.claude`, le `.gitconfig` local et les secrets sont exclus; le snapshot textuel historique déjà transféré est décrit dans [GITLAB-PC4-TRANSFER-002](../gitlab-pc4-transfer-002/local-run.md). La licence GitSpace reste indéterminée et aucun code GitSpace ou Forgejo n’est intégré au produit.

Le terminal et l’exécution de cette tâche restent sur PC6 : le nom tailnet PC4 ne résout pas dans ce shell. L’accès distant utilisé est le connecteur GitLab déjà authentifié; aucun courriel ni identifiant n’a été consulté. Le dépôt continue d’être synchronisé via l’API, et les contrôles PC4 sont observés via ses pipelines. Ce n’est pas un terminal de développement sur PC4.
