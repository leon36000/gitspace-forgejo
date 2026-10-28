# POC-032 — reprise interprocessus du PUT LFS HTTP

Date : 24 octobre 2026. Source Forgejo scratch : tag `v17.0.0`, HEAD officiel `0d791421f85ca13a63b665053d81ea765fbbc571`. Le scratch `/tmp/forgejo-v17-base-cas-poc` est déjà modifié par d'autres POC; le test reste sous build tag et aucun code de production Forgejo n'est modifié par ce lot.

## Résultat observé

Le nouveau test tagged complète POC-031 par une véritable frontière de processus. Un premier processus du binaire de tests Forgejo appelle le routeur d'intégration authentifié (`PUT /user2/<repo>.git/info/lfs/objects/<oid>/<size>`), atteint le vrai `services/lfs.UploadHandler`, puis un adaptateur de stockage exclusivement test-only réserve une génération dans une base SQLite fichier distincte, écrit le stage dans le véritable `storage.LocalStorage`, relit et vérifie taille et SHA-256/OID. À cet instant le test appelle `os.Exit(73)` depuis le chemin d'écriture, avant promotion et avant le retour du handler.

Le processus parent rouvre la même base et le même stockage : il lit une seule génération `pending`, aucune ligne `current`, et valide à nouveau les octets. Il lance ensuite un second processus de test Forgejo, qui recrée les fixtures de route, envoie le même PUT, retrouve l'opération pending, valide les octets et promeut le mapping. Le vrai GET avec `Range: bytes=0-7` renvoie `206` et le préfixe attendu. Le parent confirme que l'OperationID, la génération et la clé sont inchangés, que l'état est `active` et que l'objet se vérifie encore.

Les tables POC et l'adaptateur de génération restent hors de Forgejo. Seuls la base de générations SQLite fichier et `LocalStorage` persistent entre les deux enfants. Le harnais d'intégration recrée son DB Forgejo en mémoire pour chaque enfant et recharge ses fixtures : ce POC ne démontre donc pas la persistance du `LFSMetaObject` ni un redémarrage de service déployé. Il démontre la reprise du stage HTTP avec mapping et blob durables dans ce modèle. La commande inclut aussi les deux tests de POC-031 pour vérifier qu'ils continuent de passer après le changement de l'adaptateur.

## Vérification reproductible

Forgejo : commit officiel `0d791421f85ca13a63b665053d81ea765fbbc571`. Go `1.27.3 linux/arm64`; XORM `code.forgejo.org/xorm/xorm v1.4.1`; SQLite driver `github.com/mattn/go-sqlite3 v1.14.52`.

```sh
GOTOOLCHAIN=local GOPROXY=off GOSUMDB=off /tmp/forgejo-v17-toolchain/go/bin/go test \
  -tags 'sqlite sqlite_unlock_notify forgejo_poc_sqlite_generation_upload_route' \
  ./tests/integration \
  -run '^TestPOCLFS(AuthenticatedUploadPromotesSQLiteGenerationAndReadsItBack|UploadRetryPromotesDurablePendingGeneration|UploadSurvivesProcessRestart)$' \
  -count=1 -v
```

Résultat final : 3/3 tests passent; la commande sort avec code 0. Le test `TestPOCLFSUploadSurvivesProcessRestart` exige la sortie code 73 du processus de stage, puis le code 0 du processus de retry. Le journal complet est `final-test.log`. `gofmt -d` est vide; contrôle de blancs/conflict markers : zéro. SHA-256 du test scratch et versions : `scratch-test-source.go`, `source-review.txt`, `tool-versions.txt`.

La première tentative a réutilisé le répertoire de queue et le listener SSH de la suite d'intégration, ce qui a fait échouer le démarrage du processus enfant. Une autre tentative a hérité de `HOME` du poste et activé le contrôle SSH local; son log ne montrait que les numéros de lignes de la liste `authorized_keys`, sans afficher les clés. Le harnais enfant utilise maintenant une configuration et des chemins temporaires, désactive SSH et prend un HOME temporaire. Ces échecs concernaient uniquement l'isolation du harnais; le dernier run passe. Aucun service externe n'a été appelé et aucune clé n'a été transmise.

## Limites et suite

L'arrêt est brutal au niveau du processus de test, après stage vérifié, pas au milieu de l'écriture. Le test est mono-hôte Linux, fichier SQLite et LocalStorage; il ne couvre pas interruption matériel, base Forgejo persistante au redémarrage, concurrence simultanée du vrai routeur, hash/size invalide, stockage S3/MinIO, suppression, Doctor/GC, URL versionnée, client `git-lfs`, tous les autres writers, migrations ou nettoyage/rétention. Il ne raccorde aucun code de production Forgejo. Le coût d'un seam upstream reste à chiffrer et ce POC ne justifie pas un fork.

Prochaine étape : fermer les interactions de suppression, Doctor/GC, URL et `Resolve/Open` dans un seam générationnel typé; ensuite cartographier tous les writers, dumps/migrations et reprise des pending avant d'évaluer le coût upstream.
