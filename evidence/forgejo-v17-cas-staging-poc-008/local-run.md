# FORGEJO-CAS-STAGING-POC-008 — précondition côté serveur et staging LFS (24 octobre 2026)

## Source et portée

- Source officielle Forgejo `v17.0.0`, commit `0d791421f85ca13a63b665053d81ea765fbbc571`, clone scratch `/tmp/forgejo-v17-base-cas-poc`. Le clone était propre avant l’expérience; les changements sont restés dans ce scratch et ne sont pas copiés au produit.
- `LICENSE` indique GNU GPL version 3. L’expérience ne réutilise aucun code GitSpace; sa licence demeure indéterminée.
- Environnement observé : Linux `arm64`, Go `1.27.3`, Git `2.43.0`, SQLite. Le scratch a un diff source de 14 fichiers (`+488/-30`, tests inclus); `git diff --check` ne retourne pas d’erreur. L’empreinte du diff et les empreintes des fichiers touchés sont dans `scratch-source-sha256.txt`.

Question : peut-on imposer la comparaison de la base attendue à la frontière serveur d’écriture, tout en préparant les associations LFS sans les rendre visibles avant l’installation du ref Git ? C’est une expérience de compatibilité, pas une branche Forgejo adoptée.

## Précondition obligatoire au pre-receive

Le prototype ajoute un booléen à `PullRequestsConfig`, propage la base capturée par les writers PR merge et rebase-update, puis compare cette valeur au `oldCommitID` reçu par `preReceiveBranch`. Le chemin exige aussi un contexte serveur PR et un trigger interne reconnu; la lecture en erreur de la configuration échoue fermée. Une base absente ou périmée est rejetée avant le ref update. Les chemins « marquer manuellement fusionnée » et autodétection manuelle sont désactivés pour ce profil.

Deux tests d’intégration passent :

- `TestPullMergeExpectedBaseCommitID` active la règle dans la configuration du dépôt puis fusionne une PR via le chemin Web normal; le merge serveur transmet la base capturée et la comparaison pre-receive accepte le ref attendu.
- `TestGitPushExpectedBaseCommitIDRequired` crée un commit de base A, avance la branche à B quand la règle est désactivée, puis tente une mise à jour candidate construite depuis B avec le token A. La branche candidate est fast-forward de B, mais le token A est rejeté. Un autre push sans contexte PR serveur reste refusé même si l’environnement fournit B.

Commande utilisée :

```sh
PROJECT_ROOT=/tmp/forgejo-v17-base-cas-poc PROJECT_CONF=tests/sqlite.ini \
  /tmp/forgejo-v17-toolchain/go/bin/go test -tags 'sqlite sqlite_unlock_notify' \
  ./tests/integration \
  -run '^TestPullMergeExpectedBaseCommitID$|^TestGitPushExpectedBaseCommitIDRequired$' \
  -count=1
```

Résultat enregistré : `ok forgejo.org/tests/integration 2.429s`.

## Staging LFS

Le prototype crée des associations en attente avec une clé d’opération déterministe, le dépôt, le ref cible, la base attendue, le commit candidat et l’OID. Les lecteurs existants continuent d’interroger les seules associations actives. La promotion s’exécute dans une transaction et est idempotente; le post-receive et le retour réussi du push la rejouent. Un refus explicite supprime le stage de l’opération, et la collecte tient compte des OID en attente pour ne pas effacer un objet partagé encore référencé.

`TestLFSPendingMetaObjectLifecycle` vérifie l’invisibilité avant promotion, le doublon de stage, la double promotion, l’abandon d’un stage refusé et la conservation d’une référence pendante dans le comptage utilisé par le GC.

```sh
PROJECT_ROOT=/tmp/forgejo-v17-base-cas-poc PROJECT_CONF=tests/sqlite.ini \
  /tmp/forgejo-v17-toolchain/go/bin/go test -tags 'sqlite sqlite_unlock_notify' \
  ./models/git -run '^TestLFSPendingMetaObjectLifecycle$' -count=1
```

Résultat enregistré : `ok forgejo.org/models/git 0.411s`.

**Limite de preuve :** `git-lfs` n’est pas installé (`git lfs version` répond « not a git command »). La tentative `TestGit/HTTP/sha1/LFS` échoue au setup `git lfs install`, avant un merge; elle ne vérifie donc pas le transfert LFS de bout en bout avec Forgejo. Le modèle LFS a été testé, mais pas le parcours réel merge/rebase contenant un pointeur LFS.

La ligne d’intention est durable et la promotion peut être rejouée à partir de sa clé, mais le scratch n’ajoute pas de worker de réconciliation qui observe la branche après une coupure survenue après le ref update et avant le début du post-receive. Il manque aussi l’expiration/collecte des stages orphelins. Ce POC réduit l’effet latéral et établit une reprise idempotente du modèle; il ne démontre pas encore une récupération automatique après toute interruption.

## Compatibilité et coût de maintenance

La règle n’a pas d’interface UI/API dans le scratch : les tests modifient directement la configuration JSON du RepoUnit. Lorsqu’elle est activée, **toutes les mises à jour de branches** dépourvues du contexte PR interne, du trigger autorisé et de la base exacte sont rejetées. Cela inclut les pushes Git usuels et les écrivains de fichiers qui ne passent pas par les services PR modifiés. Seuls les chemins merge et rebase-update adaptés au protocole sont couverts; le contrôle n’est pas appliqué aux tags. Le chemin de marquage manuel est volontairement désactivé.

Le coût observé du patch est 14 fichiers et 434 lignes ajoutées, 30 retirées; ce n’est pas un chiffrage en jours. Une adoption amont demanderait encore une migration de schéma, une surface de configuration, une politique explicite pour les écrivains non-PR, des tests API/Web/AutoMerge et de toutes les variantes de rebase, ainsi qu’un reconciler LFS, le nettoyage de rétention et des tests crash/concurrence. Le point d’incompatibilité direct est le blocage des writers sans token; son périmètre est tout ref de branche d’un dépôt opt-in.

**Décision :** ne pas adopter ni déployer ce patch. Il confirme que la frontière pre-receive peut imposer un CAS aux opérations serveur qui propagent une base. Pour le pilote, conserver l’intégration par interfaces si l’autorité de merge unique et ses permissions sont imposées; si plusieurs writers doivent rester actifs, il faut d’abord concevoir et vérifier la compatibilité de la politique complète. Le diff v17 n’est ni un correctif upstream prêt, ni une preuve qu’un fork limité est préférable après coût de maintenance.

## Extension — writer API de fichiers et effet LFS (24 octobre 2026)

Un test supplémentaire traverse l’API réelle `POST /repos/{owner}/{repo}/contents/{filepath}` sur le dépôt public de fixture `user2/repo1`. Après ajout de `.gitattributes` sans politique stricte, le test active `RequireExpectedBaseCommitID` puis envoie un fichier suivi par LFS. L’API répond `403`, et la tête Git n’a pas changé. Pourtant `services/repository/files/update.go::CreateOrUpdateFile` avait déjà créé l’association `LFSMetaObject` et placé le contenu dans le stockage avant que `TemporaryUploadRepository.Push` n’atteigne le contrôle pre-receive. Le test relit l’association et confirme qu’un GET LFS anonyme sur ce dépôt public renvoie les octets synthétiques du fichier. Cette observation est circonscrite à ce chemin et à ce dépôt public; elle ne démontre pas un accès non autorisé dans un dépôt privé.

La commande groupée, après ajout de ce test, passe trois scénarios du scratch : merge normal sous règle stricte, rejet d’un push avec base périmée ou sans contexte serveur, et refus API du writer fichier avec la métadonnée LFS résiduelle. Sortie : `ok forgejo.org/tests/integration 3.647s`. Le test de modèle `TestLFSPendingMetaObjectLifecycle` repasse séparément (`ok forgejo.org/models/git 0.204s`). Commandes et sorties minimales : `strict-writer-009-test-result.txt`.

Le contrôle ajouté dans `routers/private/hook_pre_receive.go::preReceiveBranch` ne couvre que les refs branche. `assertExpectedBaseCommitID` exclut explicitement les wikis; les tags suivent `preReceiveTag`. `cmd/hook.go::runHookPreReceive` court-circuite aussi lorsque `GITEA_INTERNAL_PUSH=true`; la seule utilisation de `InternalPushingEnvironment` trouvée dans le clone est l’initialisation de dépôt (`services/repository/init.go`), mais une politique générale doit inventorier et restreindre ce bypass. Les writers de fichiers (`services/repository/files/{update,upload,temp_repo}.go`) n’ont ni token CAS ni staging LFS et sont donc incompatibles avec le profil strict actuel. D’autres opérations LFS (upload/import/synchronisation, association, fork/template) restent hors du staging expérimental.

Ce test met au jour un résidu de base de données et de stockage après rejet — différent du simple rejet du ref. Le modèle pending testé couvre seulement les associations préparées par les chemins PR merge et update-rebase; il n’est pas encore intégré à tous les writers. Il n’existe toujours pas de reconciler automatique après crash, de nettoyage à TTL, de migration ni d’interface de configuration. Le statut global reste **partiel** : la précondition bloque bien le ref pour les cas exercés, mais le protocole complet « ref + LFS » n’est pas atomique ni récupérable pour l’ensemble des routes.

Le diff expérimental passe maintenant à 14 fichiers et `+488/-30`; ce décompte ne chiffre pas les jours ni le coût récurrent de rebase. Une adoption demanderait au minimum d’étager également les fichiers API/WEB et les autres writers concernés, définir la portée tags/wiki/internal, puis tester concurrence, refus, coupure et reprise. Pas de fork ni de code Forgejo adopté au produit.

## Reproductibilité

Le commit officiel, les versions, le hash du diff scratch et les empreintes des 14 fichiers touchés sont enregistrés dans `scratch-source-sha256.txt`. Les sorties de test locales capturées sous `/tmp/forgejo-cas-integration-008.log` et `/tmp/forgejo-cas-lfs-model-008.log` avaient respectivement les SHA-256 `ba5e3672b1c524dea26c7099c3fc0af62c2c7d85b872837ba32bca4f5199c42e` et `5337e646e1888b620e26ce21561178926e7c553e867492a7eb928f100f2d5d44`; les commandes ci-dessus recréent les résultats, sans inclure de transcriptions d’authentification dans le produit.
