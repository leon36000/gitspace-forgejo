# Revue indépendante — POC-007 (24 octobre 2026)

Lecture seule du scratch `/tmp/forgejo-v17-base-cas-poc`; aucun test lancé par la revue.

La suppression conditionnelle sur `(scheduleID, pullID)` empêche l’ancien worker de supprimer la planification B lorsque A a été annulée et remplacée avant son claim. Le test `TestPullMergeStaleAutoMergeWorkerCannotConsumeReplacementSchedule` vérifie un nouvel ID et un style différent.

**Point P2 restant :** le worker supprime A lors du claim puis peut la recréer après `ErrExpectedBaseCommitRequired`. Si B est créé puis annulé avec succès (`204`) dans l’intervalle, le worker ne sait pas qu’une nouvelle génération a existé; sa restauration peut ressusciter A. La vérification `ErrAlreadyScheduledToAutoMerge` protège B uniquement s’il existe encore au moment de la restauration.

Pour fermer ce cas, conserver une génération ou un état de claim durable et conditionner la restauration à son absence de changement. Une annulation sur une opération déjà en cours doit avoir une réponse définie. Le comportement n’est ni corrigé ni vérifié par ce POC.
