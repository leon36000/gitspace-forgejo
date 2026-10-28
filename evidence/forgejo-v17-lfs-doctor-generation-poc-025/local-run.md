# POC-025 — auto-fix Doctor et chemins LFS générationnels

Date : 2026-10-24  
Scratch Forgejo : `/tmp/forgejo-v17-base-cas-poc`  
HEAD upstream : `0d791421f85ca13a63b665053d81ea765fbbc571` (`v17.0.0`)  
Go : `go1.27.3 linux/arm64`  
Test ajouté : `tests/integration/lfs_doctor_generation_path_poc_test.go`, gardé par `forgejo_poc_lfs_generation_doctor`.

## Question

Le checker Doctor de v17 peut-il supprimer un objet encore référencé si le backend expose la clé physique proposée `generation/<oid>/<generation>/<operation>` à `IterateObjects`?

## Procédure et résultat

Le test ne mocke ni `doctor.CheckStorage`, ni `commonCheckStorage`, ni le backend. Le harnais d’intégration prépare SQLite; le test crée une association LFS active avec `git_model.NewLFSMetaObject`, écrit le payload sous la clé générationnelle avec le vrai LocalStorage, puis appelle `doctor.CheckStorage({LFS:true})` avec `autofix=true`.

1. **Assertion de sécurité rouge :** la variante de test qui attendait que la clé existe après auto-fix échoue avec `stat .../generation/<oid>/1/poc-operation: no such file or directory`. Le statut est 1. La trace inclut ensuite du bruit de teardown `context canceled` du harnais après l’échec.
2. **Diagnostic vert du comportement constaté :** la variante actuelle attend l’absence de la clé physique et vérifie simultanément que l’association logique OID est toujours présente en SQLite. Le test ciblé passe (code 0).

Commande du run rouge :

```sh
GOTOOLCHAIN=local /tmp/forgejo-v17-toolchain/go/bin/go test -tags 'sqlite sqlite_unlock_notify forgejo_poc_lfs_generation_doctor' ./tests/integration -run '^TestPOCDoctorAutofixPreservesActiveGenerationKey$' -count=1 -v
```

Commande du run diagnostic :

```sh
GOTOOLCHAIN=local /tmp/forgejo-v17-toolchain/go/bin/go test -tags 'sqlite sqlite_unlock_notify forgejo_poc_lfs_generation_doctor' ./tests/integration -run '^TestPOCDoctorAutofixCurrentlyDeletesActiveGenerationKey$' -count=1 -v
```

Résultat : rouge attendu `exit=1`, puis diagnostic `PASS`, `ok forgejo.org/tests/integration 1.291s`, `exit=0`. Les traces brutes sont `doctor-generation-safe-assertion-red.log` et `doctor-generation-unsafe-behavior-green.log`; le code de test présent est `lfs_doctor_generation_path_poc_test.go`. Le fichier `doctor-generation-safe-assertion-red-source.go` conserve la variante de l’assertion de sécurité.

## Mécanisme source

`services/doctor/storage.go::CheckStorage` passe `storage.LFS` à `commonCheckStorage`. Le callback dérive `oid` en supprimant les `/` du chemin physique et consulte `git.ExistsLFSObject`; pour un chemin générationnel, le résultat ne correspond pas à l’OID DB. `commonCheckStorage` accumule le chemin lorsqu’il est jugé orphelin, puis appelle `Delete(path)` en mode auto-fix. Le test observe donc la perte du blob sans suppression de l’association logique.

## Interprétation

**Observé :** vrai code de Doctor, vraie DB SQLite d’intégration, vrai LocalStorage; le chemin générationnel est supprimé et l’association active subsiste.

**Inférence limitée :** si un futur adapter expose ses clés physiques générationnelles via le `IterateObjects` générique existant, l’auto-fix actuel peut supprimer des blobs actifs. Ce test ne démontre pas qu’un tel adapter soit intégré au produit Forgejo v17.

**Limites :** pas de mapping generation réel raccordé, pas de code de production modifié, pas de MinIO/S3, d’autres dialectes, de concurrence, de restauration ni de test complet de migration. Le correctif doit être un inventaire LFS typé lié aux références/mappings (ou un garde sûr qui refuse l’auto-fix), pas l’inférence d’OID à partir de la clé physique.
