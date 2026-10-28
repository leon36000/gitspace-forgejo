# FORGEJO-NATIVE-COMMENT-ADMISSION-001 — tentative interrompue

La première exécution du nouveau scénario a bien démarré l’instance jetable, mais le runner s’est arrêté avant d’appeler le CLI de mission. Sa vérification attendait par erreur une copie de l’exigence sous `project/requirements/`; le dépôt temporaire ne contient que les sources de l’adaptateur et Forgejo conserve l’exigence versionnée dans son dépôt Git. Le fichier `failure.json` consigne cette erreur de harnais `FileNotFoundError`; `port-cleanup-readback.json` confirme l’arrêt des processus suivis et des ports de test.

Cette tentative n’est pas une preuve de refus d’admission. Elle a guidé le correctif de la seconde exécution : la valeur versionnée est lue et comparée par l’API de contenu Forgejo avant/après le refus. Le scratch temporaire n’a pas été réutilisé ou supprimé. Aucune sortie de modèle, publication ou action de production n’a eu lieu.
