# POC-029 — réservations LFS SQLite entre processus

Date : 24 octobre 2026  
Source Forgejo auditée : v17.0.0, commit `0d791421f85ca13a63b665053d81ea765fbbc571`  
Scratch : `/tmp/forgejo-v17-base-cas-poc`  
Go : `go1.27.3 linux/arm64`  
XORM : `code.forgejo.org/xorm/xorm v1.4.1`; SQLite driver : `github.com/mattn/go-sqlite3 v1.14.52`  
Source du test scratch : `models/git/lfs_generation_mapping_poc_test.go`, SHA-256 `0348d2259292013b5a236f5b40daffd1e89c7f50413aa595dd5906c6b893e63e`.

## Question et méthode

Le mode SQLite de Forgejo (`_txlock=immediate` et `_busy_timeout=Database.Timeout`) suffit-il à allouer des numéros de génération uniques quand des processus indépendants réservent le même OID dans un fichier partagé? Le prototype ouvre des moteurs XORM indépendants avec le DSN de fichier `file:<path>?_busy_timeout=500&_txlock=immediate&mode=rwc`, dérivé du constructeur de [`modules/setting/database.go::dbConnStrWithHost`](sqlite-config-excerpt.txt). Le paramètre par défaut est `Database.Timeout = 500`; le driver v1.14.52 interprète ce paramètre en millisecondes et traduit `_txlock=immediate` en `BEGIN IMMEDIATE`. Le harnais unitaire de `models/unittest/testdb.go::CreateTestEngine` utilise lui aussi `_txlock=immediate`, mais sur une base mémoire partagée. Les extraits et les empreintes sources sont joints.

Le test n'appelle pas un writer Forgejo. Il exécute une réservation expérimentale sous transaction XORM, avec ses propres tables POC : sélection par `operationID`, lecture des générations existantes, insertion unique puis commit. Douze processus se synchronisent avant de réserver le même OID avec douze identifiants distincts. Deux autres réservent ensuite le même `operationID` pour éprouver l'idempotence. Les readbacks vérifient l'absence de générations dupliquées et une seule ligne pour l'opération répétée.

## Échec observé, puis correction expérimentale

Un premier stress du helper sans retry a lancé douze processus simultanés avec les options de fichier ci-dessus. Le journal exploratoire conserve une erreur `database is locked` dans 9 des 10 itérations. La source de cette première version n'a pas été sauvegardée avec son empreinte au moment du run; ce journal est un signal diagnostique, pas une preuve strictement reproductible.

Le test épinglé `TestPOCConfiguredSQLiteBusyTimeoutSurfacesSingleAttemptFailure` rend le refus reproductible : un processus parent conserve une transaction d'écriture immédiate pendant 800 ms; un enfant essaie une seule réservation sous un timeout de 500 ms. L'enfant reçoit `begin reservation: database is locked`, aucune ligne n'est écrite, et le test capture ce refus comme résultat attendu. Ce scénario passe 3/3 comme test négatif contrôlé.

Le test frère `TestPOCBoundedRetryAfterConfiguredSQLiteLock` applique au helper scratch un retry borné uniquement pour `database is locked`/`database is busy`, avec délais exponentiels plafonnés et le même `operationID`. Sous le même verrou de 800 ms, le premier essai échoue après le timeout configuré et le second réussit après libération; le log contient `attempts=2`. Ce scénario passe 3/3. Dans le stress multi-processus avec le retry, les douze réservations distinctes et le doublon d'opération passent 10/10, soit 120 réservations distinctes et 10 vérifications de doublon sans collision.

Vérifications enregistrées :

- `multiprocess-bounded-retry-repeat-10.log` : 10/10 stress multi-processus réussis.
- `single-attempt-red-green-repeat-3.log` : 3/3 refus single-attempt attendus et 3/3 reprises bornées réussies.
- `models-git-all-poc-tests.log` : les cinq tests POC de `models/git` passent; l'échec du processus enfant est une assertion négative attendue.
- `gofmt-diff.log` et `scratch-diff-check.log` : vérifications de formatage et d'espaces du fichier expérimental.

## Portée et limites

Cette preuve établit que le helper scratch peut être rendu reprenable sous contention SQLite multi-processus en utilisant des identifiants d'opération idempotents et une politique de retry bornée. Elle montre aussi qu'un seul essai peut échouer malgré les options SQLite configurées lorsque le verrou dépasse le timeout.

Le retry n'est ni un changement de Forgejo ni une politique recommandée pour sa base sans revue supplémentaire. Le schéma et le helper sont exclusivement test-only; aucun `models/db.WithTx`, writer LFS de production, upload, stockage objet, promotion, suppression, Doctor/GC, migration ou route HTTP n'est relié à ce test. La preuve utilise un seul hôte Linux, SQLite fichier et XORM; elle ne couvre ni plusieurs hôtes, PostgreSQL/MySQL, NFS, ni stockage distant. La première série rouge 9/10 n'a pas de snapshot source exact, comme indiqué ci-dessus.

Les options SQL sont établies à partir du source v17 local; elles ne démontrent pas la provenance de l'instance Forgejo locale. Aucun code de production Forgejo n'a été copié ou modifié, aucun fork n'est adopté et aucun coût de maintenance upstream n'est mesuré. Le test expérimental ne change pas la décision d'architecture : Forgejo reste le socle candidat avec intégration par interfaces tant qu'aucune capacité n'exige un fork démontré.

## Suite utile

Faire traverser l'allocation, l'écriture d'objet sous clé générationnelle, la reprise après interruption et la promotion par un même scénario multi-processus; vérifier ensuite les call sites réels d'upload/delete/GC dans le scratch sans toucher aux sources de production. Avant tout fork, mesurer la compatibilité des writers et le coût de rebase. Le parcours produit avec LLM et la comparaison contrôlée avec Forgejo + agent + CI conventionnelle restent non démontrés.
