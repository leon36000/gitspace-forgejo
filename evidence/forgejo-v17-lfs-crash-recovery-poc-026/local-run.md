# POC-026 — reprise d’un stage LFS après arrêt de processus

Date : 24 octobre 2026  
Scratch Forgejo officiel : /tmp/forgejo-v17-base-cas-poc  
Révision de base : v17.0.0, 0d791421f85ca13a63b665053d81ea765fbbc571  
Go : go1.27.3 linux/arm64  
Test tagué : services/lfs/versioned_download_sqlite_poc_test.go  
Empreinte du test exécuté : 82e7849a22d731b4f749371f845448421a6e0d3f841b0b1b2b55bf7e1e724640.

## Question

Un upload dont l’objet et la réservation sont persistés avant l’activation du mapping peut-il être repris après la fin abrupte du processus, sans publier un contenu incomplet ou activer une seconde fois une génération?

## Procédure et constat

Le test sous le build tag forgejo_poc_sqlite_generation_download utilise une base SQLite dédiée et le vrai LocalStorage. Le parent relance le même binaire de test comme processus enfant. L’enfant réserve l’opération crash-before-promotion, écrit l’objet à sa clé de génération unique, puis appelle os.Exit(73) avant la transaction d’activation. Le parent exige ce code de sortie, rouvre la base et le stockage, constate la rangée pending et l’absence de mapping courant, puis appelle la récupération.

La récupération relit l’objet, vérifie taille et SHA-256 attendus, et promeut la génération dans le mapping SQLite. Une seconde récupération de la même opération ne change pas le résultat. Le readback confirme que le mapping actif et la rangée active désignent la clé générationnelle persistée avant l’arrêt.

Commande exécutée depuis la copie scratch :

    GOTOOLCHAIN=local /tmp/forgejo-v17-toolchain/go/bin/go test -tags 'sqlite sqlite_unlock_notify forgejo_poc_sqlite_generation_download' ./services/lfs -count=1 -v

Résultat : les deux tests présents dans le paquet sous ce build tag passent, dont TestPOCInterruptedLFSGenerationStageRecoversAfterProcessRestart; ok forgejo.org/services/lfs 0.276s. gofmt -d et git diff --check ne signalent aucun écart. Journaux complets, version, source exacte et empreintes sont conservés à côté de ce rapport.

## Interprétation et limites

**Démontré dans ce POC :** reprise après terminaison d’un processus utilisateur, persistance du stage SQLite + LocalStorage, absence de mapping actif avant promotion, vérification des octets avant reprise et idempotence séquentielle de la récupération.

**Non démontré :** coupure d’alimentation ou durabilité matérielle, panne au milieu d’une écriture, récupération concurrente depuis plusieurs processus, course avec suppression, garanties de transaction sur PostgreSQL/MySQL, backend S3/MinIO, URLs signées, GC/Doctor, dump/migration ou intégration au routeur Forgejo. Le schéma, l’adapter et la récupération sont uniquement des fixtures de test; le vrai handler ne consomme pas encore ce registre. Ce résultat n’est pas une modification de production Forgejo et ne justifie pas un fork.

Le POC confirme qu’un journal durable d’opération et une clé immuable peuvent rendre reprenable le point précis « stage écrit, mapping pas encore activé ». La prochaine vérification utile porte sur deux récupérations concurrentes, collisions d’allocation et nettoyage conditionnel; le contrat devra également empêcher Doctor et GC génériques de supprimer ou d’omettre des clés dont ils ne comprennent pas le mapping.
