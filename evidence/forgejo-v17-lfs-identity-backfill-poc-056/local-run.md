# POC-056 — migration/backfill de l’identité des opérations LFS pending

Date : 25 octobre 2026. Environnement : copie scratch de Forgejo `v17.0.0`, base officielle `0d791421f85ca13a63b665053d81ea765fbbc571`, Go `1.27.3 linux/arm64`, SQLite, accès Go hors ligne (`GOPROXY=off`). Aucun code Forgejo n’a été copié dans le dépôt produit. Les empreintes des deux nouveaux fichiers, du modèle LFS et du récupérateur sont dans `source-hashes.txt`; `scratch-status.txt` montre les fichiers de ce POC avec les modifications scratch préexistantes.

## Comportement ajouté dans le scratch

La migration expérimentale `v17b_lfs_pending_meta_operation_identity.go` synchronise d’abord la table d’en-tête d’opération, car Forgejo lance ses migrations avant `SyncAllTables`. Si l’ancienne table d’associations pending n’existe pas, elle s’arrête après la création de l’en-tête. Sinon, elle parcourt les associations par ID en pages de 128, valide les identités, refuse les métadonnées contradictoires et écrit les en-têtes de chaque page dans une transaction. Elle ne supprime ni ne modifie les associations pending. Une page déjà validée peut rester écrite si une page plus tardive échoue; une relance vérifie ces en-têtes et refuse toujours la donnée contradictoire.

## Vérifications

- `GOTOOLCHAIN=local GOPROXY=off /tmp/forgejo-v17-toolchain/go/bin/go test -tags sqlite ./models/forgejo_migrations -run '^TestBackfillLFSPendingMetaOperationIdentity' -count=1 -v -timeout=10m` — 5/5 tests ciblés réussis. Les cas couvrent 129 lignes avec une opération traversant la limite de page, la répétition idempotente, le conflit intra-page avec rollback, le conflit inter-pages préservant les pending et bloquant toujours à la reprise, l’absence de table legacy, et l’exécution via `Migrate` avec enregistrement de l’ID v17b et `EnsureUpToDate`.
- `GOTOOLCHAIN=local GOPROXY=off /tmp/forgejo-v17-toolchain/go/bin/go test -tags sqlite ./models/forgejo_migrations ./models/git ./services/pull -count=1 -timeout=20m` — les trois paquets passent.
- `/tmp/forgejo-v17-toolchain/go/bin/gofmt -d ...` — aucune différence de formatage pour les deux nouveaux fichiers.

Les sorties brutes sont `targeted-tests.log`, `package-tests.log` et `gofmt-diff.log`. Pendant la mise au point, le test inter-page a d’abord révélé que XORM `Session.Get` considère chaque champ non vide comme une condition. La recherche portait donc aussi sur la ref contradictoire et tombait à faux avant une insertion refusée par l’unicité. La migration cherche maintenant avec la seule clé d’opération, puis compare les valeurs; l’observation console initiale est retranscrite et explicitement qualifiée dans `initial-failure-observation.txt`.

## Limites et statut

Un test passe par le dispatcher réel `Migrate`, mais avec un registre temporairement isolé où le seul ID enregistré est `v17b_lfs_pending_meta_operation_identity` et où son `Upgrade` est la fonction testée. Il vérifie aussi la ligne de migration complétée et `EnsureUpToDate`. Il ne démarre pas Forgejo et n’exerce pas la base v17a avec le registre complet des migrations. Les tables legacy simulées et les modèles sont testés seulement sur SQLite; ni PostgreSQL/MySQL, processus concurrents, crash au milieu d’une page, ni effets Git ne sont couverts. Cela ne prouve pas la publication LFS active, un worker de reprise, un checkpoint durable, une fairness, la rétention ou le nettoyage. La migration et les changements Forgejo restent expérimentaux, non adoptés, non proposés upstream et non copiés au produit.

Suite suggérée : concevoir un checkpoint de balayage persistant à curseur et budget borné, avec reprise après interruption, fairness et revisite des opérations qui échouent; établir ensuite la rétention/nettoyage. Avant un worker ou un fork, tester le cycle de vie réellement démarré et les writers qui contournent ce seam.
