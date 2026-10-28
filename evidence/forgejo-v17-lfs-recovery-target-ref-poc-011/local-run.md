# FORGEJO-LFS-RECOVERY-TARGET-REF-POC-011 — liaison de la reprise à la ref cible

## Source et question

Source officielle Forgejo `v17.0.0`, commit `0d791421f85ca13a63b665053d81ea765fbbc571`, dans le scratch isolé `/tmp/forgejo-v17-base-cas-poc`. Environnement : Go `1.27.3 linux/arm64`, Git `2.43.0`. Aucun fichier Forgejo n’a été copié au dépôt produit. Ce POC ne porte que sur la fonction de reprise expérimentale `services/pull/lfs_recovery.go::RecoverLFSPendingMetaObjects` et ses tests.

La version POC-010 cherchait `for-each-ref --contains=<candidat>` sans limiter la recherche à `TargetRef`. Un candidat présent sur une branche sans rapport pouvait donc promouvoir un stage créé pour une autre branche. Le test rouge `TestRecoverLFSPendingMetaObjectsDoesNotPromoteFromUnrelatedRef` a observé `PromotedOperations=1` alors que le résultat attendu était zéro, parce que le commit n’était que sur `refs/heads/source`, et pas sur `refs/heads/main`.

## Correction expérimentale et test

La récupération passe maintenant `operation.targetRef` comme motif à `git for-each-ref`, puis ne retient que la ligne dont le nom est strictement égal à cette ref. L’égalité exacte est nécessaire, car les motifs `for-each-ref` peuvent inclure des noms ayant ce préfixe. Le test final place donc le commit seulement sur `refs/heads/main-shadow`, tandis que le stage cible `refs/heads/main`; il vérifie que la métadonnée reste invisible et que le stage est conservé. Le test POC-010 de reprise après avancement de la ref cible continue également de passer.

**Sémantique choisie :** ce stage est une intention par opération et sa reprise ne promeut que lorsque la ref qu’il nomme atteint le candidat ou un descendant qui le contient. Cette fonction ne constitue pas un verdict de PR, une autorisation de fusion ou une preuve de correction du code. La métadonnée LFS Forgejo est toutefois au niveau du dépôt : un objet utilisé depuis une autre ref peut justifier sa disponibilité globale au dépôt. Le choix cible-ref privilégie la preuve de l’opération et peut laisser un stage pending tant que la ref ciblée ne rejoint pas le candidat; une éventuelle réconciliation repo-wide doit être spécifiée séparément.

## Vérifications reproductibles

Le test rouge ci-dessus a été observé avant le changement du prédicat. Après correction, la commande suivante exécute sans cache tout le paquet touché, y compris reprise normale, candidat absent et branche de même préfixe :

```sh
cd /tmp/forgejo-v17-base-cas-poc
PROJECT_ROOT=/tmp/forgejo-v17-base-cas-poc PROJECT_CONF=tests/sqlite.ini \
  /tmp/forgejo-v17-toolchain/go/bin/go test -count=1 \
  -tags 'sqlite sqlite_unlock_notify' ./services/pull
```

Résultat : `ok forgejo.org/services/pull 0.945s`. Sortie conservée dans `services-pull-test.log`. `gofmt -d` et `git diff --check` sont propres dans le scratch. Les SHA-256 des 16 fichiers touchés sont dans `scratch-source-sha256.txt`; aucun autre fichier source n’a été modifié pour ce POC.

## Limites et conséquences d’intégration

La fonction n’a aucun appelant de production; aucun worker ou redémarrage Forgejo n’est testé ici. Le test utilise un dépôt bare temporaire, pas un serveur Forgejo vivant ou `git-lfs`. La récupération ne valide pas que chaque OID staged se trouve dans l’arbre du commit candidat. Elle n’est pas bornée/paginée (`models/git/lfs.go::GetLFSPendingMetaObjectsByRepository`), n’exécute pas de récupération simultanée, ne couvre pas la suppression de refs, les dépôts privés, le TTL ou l’annulation.

L’inventaire en lecture seule identifie également un risque avant adoption : `services/repository/delete.go:279–300` ne compte et ne supprime que `LFSMetaObject`, puis `:375–378` peut supprimer le blob. Il ne compte ni ne supprime les lignes `LFSPendingMetaObject`. Supprimer un dépôt qui porte la dernière association active peut donc enlever le contenu commun alors qu’un autre dépôt le retient seulement en pending; une reprise ultérieure ne recréera pas le blob. Ce chemin est constaté dans le code, pas reproduit par un test de suppression.

Le profil CAS POC-008 reste incompatible avec plusieurs writers. Activé, il refuse les mises à jour de branches sans contexte et base de PR serveur; les tags/wikis et push interne sont hors règle. Les routes fichier (`services/repository/files/update.go:440`, `upload.go:142`) créent déjà une association active avant le push; le refus HTTP exercé laisse cette association dans la fixture publique. L’upload LFS (`services/lfs/server.go:244,350`) et la synchronisation/import (`modules/repository/repo.go:197,244`) créent aussi des associations en amont, ce qui suit le protocole de transfert courant mais ne respecte pas une règle « aucune association avant ref ». Le patch expérimental courant couvre 14 fichiers suivis (`+499/-30`) et deux fichiers nouveaux (244 lignes); cela chiffre seulement le code prototype, pas le coût des adaptations manquantes ni les jours/rebases upstream. Le coût réel inclut un contrat de compatibilité par writer, migration/configuration, worker, suppression/GC et suites de régression à chaque mise à niveau.

**Décision :** conserver le prototype dans le scratch sans l’adopter. Garder Forgejo comme candidat de socle Git avec intégration par interfaces; le fork ne devient justifié que si une exigence multi-writer prouve la nécessité d’une précondition serveur et si le protocole complet couvre writers, droits LFS, suppression, récupération et rétention.
