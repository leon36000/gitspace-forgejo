# MERGE-006 — compatibilité Forgejo de la borne 64 octets

Vérification locale en lecture seule, le 2026-10-22, dans le dépôt officiel Forgejo récupéré depuis `https://codeberg.org/forgejo/forgejo.git` à la révision `ea9ae1e174ac290febcd92f57e7a28ccb58262d0`.

Commandes d’identification et de vérification d’état exécutées :

```text
git -C /tmp/forgejo-audit-merge006 rev-parse HEAD
git -C /tmp/forgejo-audit-merge006 status --porcelain
```

La première renvoie `ea9ae1e174ac290febcd92f57e7a28ccb58262d0`; la seconde ne produit aucune ligne. Les sources directes observées à ce commit établissent :

- `modules/structs/pull_review.go::ReviewStateType` liste `APPROVED`, `PENDING`, `COMMENT`, `REQUEST_CHANGES`, `REQUEST_REVIEW` et la chaîne vide. Le plus long, `REQUEST_CHANGES`, compte 15 octets ASCII. [Source épinglée](https://codeberg.org/forgejo/forgejo/src/commit/ea9ae1e174ac290febcd92f57e7a28ccb58262d0/modules/structs/pull_review.go#L10-L25).
- `models/issues/review.go::PullReview.CommitID` est déclaré `string` avec `xorm:"VARCHAR(64)"`. [Source épinglée](https://codeberg.org/forgejo/forgejo/src/commit/ea9ae1e174ac290febcd92f57e7a28ccb58262d0/models/issues/review.go#L134-L140).
- `modules/git/object_format.go::Sha1ObjectFormatImpl.FullLength` retourne 40 et `Sha256ObjectFormatImpl.FullLength` retourne 64. [Source épinglée](https://codeberg.org/forgejo/forgejo/src/commit/ea9ae1e174ac290febcd92f57e7a28ccb58262d0/modules/git/object_format.go#L63-L67) et [SHA-256](https://codeberg.org/forgejo/forgejo/src/commit/ea9ae1e174ac290febcd92f57e7a28ccb58262d0/modules/git/object_format.go#L102-L106).

Cela justifie la compatibilité observée d’une borne locale de 64 octets pour les champs DTO modélisés `state` et `commit_id`. Ce contrôle n’est pas une limite du protocole Forgejo, ne plafonne pas le JSON avant désérialisation et ne borne pas les autres champs. Le futur lecteur HTTP doit limiter le corps et les octets cumulés avant construction du snapshot. Un résultat signé ou haché attesterait l’intégrité de son entrée, pas l’exactitude de l’approbation.
