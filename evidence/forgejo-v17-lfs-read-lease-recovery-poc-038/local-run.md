# POC-038 — expiration et reprise de leases de lecture LFS

Date : 24 octobre 2026, heure locale de Montréal. Baseline Forgejo officiel : `v17.0.0`, commit `0d791421f85ca13a63b665053d81ea765fbbc571`. L’expérience est ajoutée uniquement au scratch `/tmp/forgejo-v17-base-cas-poc`; aucun code Forgejo n’est copié dans le dépôt produit ni proposé à upstream.

## Question et mécanisme éprouvé

POC-037 a montré qu’un lease durable retient correctement un blob pendant un GET, mais qu’un processus lecteur arrêté laisse le lease actif indéfiniment. Ce test isolé introduit une table SQLite de leases avec échéance et les transitions candidates suivantes : `active` → `delete_claimed` → `deleted`.

Le renouvellement est un compare-and-set : il ne réussit que si le lease est encore `active` et `expires_at_ms > now`. La revendication GC passe le mapping générationnel de `retired_pending_delete` à `delete_claimed` avec un `UPDATE` SQL unique qui exige l’absence de lease actif non expiré. Le test vérifie qu’un lecteur vivant peut renouveler, que son ancien échéancier ne bloque plus à la nouvelle échéance, que le renouvellement après échéance échoue, et que le worker peut reprendre après arrêt survenu après la suppression physique mais avant le readback DB. La suppression répétée est tolérée pour la même clé immuable.

## Résultats reproductibles

Outil : Go `1.27.3 linux/arm64`, SQLite du harnais local et `LocalStorage`; les téléchargements de modules et sommes sont désactivés (`GOPROXY=off`, `GOSUMDB=off`). La source du test, son SHA-256 et les sorties sont archivés dans ce répertoire; `tool-versions.txt` fixe la référence produit et Forgejo.

Test ciblé :

```sh
GOTOOLCHAIN=local GOPROXY=off GOSUMDB=off /tmp/forgejo-v17-toolchain/go/bin/go test \
  -tags 'sqlite sqlite_unlock_notify forgejo_poc_generation_streamed_download forgejo_poc_sqlite_generation_download' \
  ./services/lfs -run '^TestPOCExpiredReadLeaseCanBeReclaimedAfterProcessRestart$' -count=1 -v
```

Résultat : 1 test réussi (`focused.log`). Le paquet `./services/lfs` complet avec les mêmes tags passe (`package.log`). `gofmt -d` est vide et le contrôle d’espaces du nouveau fichier passe.

## Limites qui empêchent toute adoption

Ce test est un modèle SQLite/stockage local, pas un changement de production Forgejo. Il ne branche pas le renouvellement à `DownloadHandler`, ne renouvelle pas pendant un vrai stream, n’annule pas la réponse quand le lease est perdu, et n’interlace pas une lecture lente avec le GC. Donc il ne prouve pas qu’un lecteur actif survit à une pause au-delà du TTL. Un TTL seul peut effacer un objet sous un lecteur lent; il faut fixer une durée maximale de transfert, renouveler avec une marge et interrompre le stream lorsque le renouvellement échoue. La cohérence des horloges entre processus doit aussi être conçue.

La récupération suppose des clés physiques uniques par génération et des suppressions idempotentes. Le test ne couvre pas S3/MinIO, erreurs/retries de backend, concurrence multi-processus, Doctor/GC global, liens batch/presign, migration, dump/restauration ni client `git-lfs`. Le protocole candidat réclame ensuite un test de course entre renouvellement, revendication et lecture ainsi qu’un contrat de stockage/reprise; aucune compatibilité universelle n’est affirmée.

Ce POC affine la conception de récupération, mais ne résout ni le coût de maintenance Forgejo, ni le choix fork/intégration. Aucun parcours LLM ou gain comparatif n’est établi ici.
