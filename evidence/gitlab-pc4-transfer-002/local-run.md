# GITLAB-PC4-TRANSFER-002 — Synchronisation du snapshot texte PC6

Date : 28 octobre 2026 (America/Montreal).

## Cible

Le projet privé GitLab PC4 est root/gitspace-forgejo (ID 4). L’intégration GitLab PC4 est en version 19.5.1, révision 449ed2875c2. La branche de travail est codex/pc6-transfer-work-2026-10-28; elle n’est ni la branche par défaut ni protégée, et l’API confirme can_push=true. La branche par défaut protégée codex/pc6-transfer-2026-10-28 n’a pas été modifiée.

Le snapshot local vient du worktree PC6, branche local/WEBHOOK-005-canonical-links, HEAD f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0. Le dernier HEAD GitLab au moment du premier readback complet est 047d66ec07e678b74093b7b0a187ae224dcfb919.

## Transfert réalisé

Les 134 fichiers de sources, tests, scripts, manifeste Cargo et documentation sont directement présents sur la branche de travail. Avant la synchronisation finale de ce checkpoint, 133/134 blobs de ce noyau correspondaient déjà; le seul écart observé était docs/checkpoint.md, qui est mis à jour dans le commit de reçu suivant.

Les preuves texte historiques sont regroupées dans evidence/gitlab-pc4-transfer-002 : archive tar/gzip déterministe de 14 887 fichiers, 60 571 312 octets avant compression, 12 979 847 octets compressés, empreinte SHA-256 e9b0c75d2228bac0aa68221250f35e5677e5f30b7dc2809ad18b06ce49a4705c. L’archive est encodée en 68 fichiers Base64 texte totalisant 17 306 464 caractères. Le fichier CONTENTS.jsonl contient la taille et le SHA-256 de chaque membre, et a aussi été publié à côté de l’archive.

Le décodage local des 68 parties a reconstruit exactement l’archive. Chaque membre a été relu depuis le tar, comparé à son fichier source et vérifié par SHA-256; les chemins absolus et traversées « .. » ont été refusés. Le checksum index evidence/SHA256SUMS est transféré directement; il couvre les nouveaux fichiers de reçu et les parties de l’archive.

## Vérifications distantes

Sur le HEAD 047d66ec, le readback récursif GitLab a retourné 255 entrées de tree, dont 234 blobs. Les 82 blobs synchronisés dans cette étape (sources divergentes, deux reçus Release, fichiers de contrôle/manifestes et 68 parties) correspondent aux hash de blob et aux modes calculés localement : 0 absent, 0 divergence. La comparaison des 134 chemins noyau n’avait alors que le checkpoint en attente; le readback final sera consigné après sa synchronisation.

Le projet ne contient pas de fichier .gitlab-ci.yml dans l’arbre transféré. GitLab lance toutefois une configuration de pipeline côté service, avec pc4_gitleaks, pc4_semgrep, pc4_trivy et pc4_ci_fallback. Les pipelines de push antérieurs ont été annulés automatiquement par les poussées suivantes. Le dernier pipeline observé au HEAD de contenu 047d66ec était en attente; le résultat du pipeline final sera ajouté après le reçu.

## Prévol, exclusions et limites

Le balayage ciblé courant a examiné 15 022 fichiers texte candidats (71 599 153 octets), avec 0 correspondance aux motifs connus pour PAT GitLab/GitHub, jetons Runner, clés OpenAI/AWS, PEM privé, identifiants URL et Bearer. Ce balayage ne garantit pas l’absence de formats de secrets non reconnus. Le prévol plus large précédent a examiné 15 080 fichiers réguliers et 2 712 membres ZIP, sans correspondance ni archive illisible.

Les fichiers binaires, 28 snapshots ZIP, objets Git embarqués, code Go des POC Forgejo, caches/builds, .runtime et .claude ne sont pas transférés. Les droits GitSpace restent indéterminés; aucun code GitSpace ou Forgejo n’a été copié dans le noyau. L’historique de commits, tags et refs du dépôt local PC6 n’est pas préservé : l’API GitLab a créé des commits de snapshot sur la branche de travail, et les refs locales restent inchangées.

Aucun mot de passe ou jeton n’a été lu dans le courriel. L’accès a utilisé l’intégration GitLab déjà connectée. Aucun merge request, merge, changement de branche par défaut, publication ou déploiement n’a été effectué.

Le runtime Codex demeure sur PC6; GitLab PC4 est maintenant la branche distante de collaboration. Les tests natifs restent exécutés localement, puis leurs sources et reçus sont synchronisés via l’intégration GitLab.
