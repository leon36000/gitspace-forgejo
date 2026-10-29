# Complément de source Forgejo — GET d’issue et sérialisation

Date : 2026-10-28. Source : checkout scratch `/tmp/forgejo-v17-base-cas-poc`, HEAD `0d791421f85ca13a63b665053d81ea765fbbc571` (Forgejo officiel v17.0.0).

La lecture au commit relie la forme JSON du test à son producteur :

1. `routers/api/v1/repo/issue.go::GetIssue` traite `GET /repos/{owner}/{repo}/issues/{index}`, appelle `GetIssueWithAttrsByIndex`, puis sérialise `convert.ToAPIIssue` avec HTTP 200.
2. `services/convert/issue.go::ToAPIIssue` délègue à `toIssue`. Celui-ci charge le dépôt et son propriétaire, puis construit `api.RepositoryMeta` avec `Owner: issue.Repo.OwnerName`, `Name: issue.Repo.Name` et `FullName: issue.Repo.FullName()`.
3. `modules/structs/issue.go::RepositoryMeta` expose ces valeurs sous `owner`, `name` et `full_name`; `Issue.Repo` expose l’objet sous `repository`.

Fichiers, Git blob IDs et SHA-256 calculés depuis les blobs du commit :

| Fichier | Git blob | SHA-256 |
| --- | --- | --- |
| `modules/structs/issue.go` | `07ff3079de602403c23dfb2530ff5ee46fb0e747` | `e6cb7890f1d7045b2423465d1e452b6f91c4d71c746957aefc9bade286d10006` |
| `services/convert/issue.go` | `c7803794d0fb475e8677363b825397c13a009f99` | `a85ec9a4567fb0a87b0af7756f3ddb2366c54375e9ad7471046c9c2fa4fd4776` |
| `routers/api/v1/repo/issue.go` | `6f573c7e1f7e1009e6bab89ed0424eaa45e92f22` | `c88041dc09cdc1097549f00d25fc5a33aefee443c21b5be2ce2e4198b4dcf576` |

`git diff --quiet <commit> -- <three paths>` returned 0 in the scratch, confirming only these files were clean against the exact commit. This is a targeted static source trace, not a build, test of Forgejo Go code, full repository audit, or observation of a live endpoint. The local adapter test remains a synthetic HTTP fixture and no upstream source was copied.
