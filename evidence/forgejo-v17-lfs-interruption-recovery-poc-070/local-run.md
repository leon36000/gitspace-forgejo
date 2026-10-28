# FORGEJO-V17-LFS-INTERRUPTION-RECOVERY-POC-070 — résultat local

Date : 25 octobre 2026. Le test s’exécute dans le scratch Forgejo v17.0.0 basé sur le commit officiel `0d791421f85ca13a63b665053d81ea765fbbc571`. Le scratch comporte de nombreux changements diagnostiques non commités; aucun code Forgejo n’est copié dans le produit.

## Question vérifiée

Après qu’un commit API a avancé la ref, le serveur évite-t-il un succès mensonger si la promotion LFS du post-receive est interrompue, et l’opération peut-elle être reprise sans exposer les détails internes au client ?

## Résultat

Le test d’intégration utilise la vraie route HTTP de création de fichier, la vraie écriture Git et la base SQLite du harnais Forgejo. Un seam de test saute la promotion post-receive. Le premier test-first run reçoit `500`, mais le corps API contient `message: ""` : la règle de production masque l’erreur interne et le client ne peut pas identifier le commit accepté ni l’opération. Cette sortie rouge/code 1 est conservée dans `red-public-message-suppressed.log` et `red-exit-status.txt`.

Le scratch ajoute `LFSPublicationPendingError`, avec un message public limité au SHA, à la ref et à l’identifiant d’opération; les détails storage/DB restent dans le journal serveur. Un test vérifie le corps API pour le code stable `lfs_publication_pending`, le SHA et l’instruction de relire la ref avant toute répétition. L’autre utilise un trigger SQLite pour forcer l’échec d’insertion des métadonnées, puis vérifie que le corps n’expose pas le texte de la cause DB. Dans les deux cas l’API renvoie `500`, la ref contient le candidat, l’association LFS reste `pending` et le GET retourne `404`. La reprise manuelle promeut une opération; le GET retourne ensuite `200` avec les octets originaux exacts.

Run final ciblé : `PASS`, 2 tests, 0 échec, 0 ignoré; paquet `tests/integration` en 2,144 s; code de sortie 0. Toolchain : Go `1.27.3 linux/arm64`, Git `2.43.0`, SQLite; Go modules hors ligne. Une itération intermédiaire a seulement échoué parce que l’assertion de test attendait le message interne au lieu du nouveau message public; ce journal est conservé dans `test-expectation-correction.log` et a été corrigé avant le run final.

## Interprétation et limites

Ce run confirme le comportement du scratch courant après la preuve POC-069 : l’état `pending` empêche la réponse `201` quand le post-receive n’a pas publié les métadonnées. Une réponse `500` peut néanmoins suivre l’acceptation du commit; le corps API indique maintenant au client de relire la ref et évite une répétition aveugle. L’identifiant d’opération est un identifiant de support, pas une API de récupération; le test appelle `RecoverLFSPendingMetaObjects` directement.

L’interruption du hook est simulée par un seam qui saute la promotion; le test ne tue pas Forgejo et n’exerce pas de worker de reprise automatique. Le second cas force bien une erreur SQLite lors de la promotion; le test exerce la reprise manuelle, pas une reprise durable après redémarrage. Les deux couvrent une instance SQLite locale et un seul chemin API; ils ne prouvent ni les autres écrivains Git/ref, ni PostgreSQL/MySQL, stockage distant, multi-hôte, ni la compatibilité ou le coût complet d’un fork. Une itération intermédiaire a eu une assertion de test trop stricte sur le message interne; elle est conservée dans `test-expectation-correction.log`, puis l’attente a été limitée au message public stable. Le logger de test marque les `log.Error` attendus comme `FATAL ERROR` dans sa sortie, bien que le run final termine avec `PASS` et code 0; le journal n’est donc pas exempt de signaux d’erreur.

Aucun modèle, service externe, instance de production ou déploiement n’est utilisé. Aucun code du scratch n’est adopté. Les commandes, journaux, toolchain, empreintes et manifeste local sont conservés dans ce dossier.
