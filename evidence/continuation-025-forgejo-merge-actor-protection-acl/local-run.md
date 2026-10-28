# CONTINUATION-025 — contrôle des protections par l’identité merge-whitelisted

Date : 26 octobre 2026. Produit `agent-forge-core`, worktree `/home/pc6/Documents/Codex/2026-10-21-mission-construire-une-forge-con-ue/.runtime/webhook-005-workspace`, branche `local/WEBHOOK-005-canonical-links`, HEAD produit `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. Aucun fichier de source Forgejo n’est copié dans le produit.

## Question et préparation

Après POC-076/077, restait une vérification de rôle distincte : un collaborateur `write` que Forgejo annonce capable de merger peut-il changer ou supprimer la règle de protection elle-même? Le code officiel v17.0.0 enregistre `/branch_protections/{name}` sous `reqToken(), reqAdmin()` (`routers/api/v1/api.go:902-910`), dont le commentaire décrit les droits owner/repo-admin (`:353-359`). Les objets consultés sont épinglés dans `source-objects.txt`; `git diff --quiet HEAD -- routers/api/v1/api.go routers/api/v1/repo/branch.go` réussit dans le scratch.

J’ai ajouté uniquement un test d’intégration scratch, `/tmp/forgejo-v17-base-cas-poc/tests/integration/merge_whitelist_protection_policy_poc_test.go`, dans le checkout déjà très modifié au commit officiel `0d791421f85ca13a63b665053d81ea765fbbc571`. Il crée un collaborateur non-admin `write`, le place seul dans la merge whitelist de la branche par défaut et lit `UserCanMerge=true`/`UserCanPush=false` sur cette même identité. Un test antérieur POC-076 a prouvé séparément la fusion API réussie de ce rôle.

## Résultat

Le même acteur tente `PATCH /branch_protections/master` pour passer `enable_push=true` et `enable_merge_whitelist=false`, puis `DELETE /branch_protections/master`. Les deux réponses sont `403`, émises par le middleware `reqAdmin`. Le propriétaire relit la règle : push toujours interdit, merge whitelist toujours active et limitée à cet acteur. Le readback de la branche conserve les mêmes indicateurs de permission; le SHA de base relu avant/après est identique.

La garde de l’adaptateur a aussi été vérifiée sur les mêmes capacités : `merge_policy_reader_allows_only_the_exact_branch_protection_get` refuse PATCH/DELETE et n’envoie que le GET allowlisté; `merge_authority_policy_rejects_other_writes_before_network_or_credential_read` refuse ces deux mutations avant lecture du credential et avant réseau. Chaque test passe 1/1. Aucun code runtime produit n’a changé dans ce volet, seulement ces assertions de refus.

Commande finale :

```sh
GOTOOLCHAIN=local GOPROXY=off /tmp/forgejo-v17-toolchain/go/bin/go test -count=1 -v -timeout=20m -tags sqlite ./tests/integration -run '^TestPOCMergeWhitelistActorCannotEditOrDeleteBranchProtection$'
```

Résultat 1/1, code 0 dans `test-final.log`; le résumé assaini des routes est dans `test-summary.log`. Le premier build du test a échoué seulement parce que deux imports inutilisés étaient présents dans le nouveau harnais; `compile-error.log` est conservé, les imports ont été retirés, puis le test ciblé final a passé. `gofmt -d` est vide (`gofmt-check.log`). Le serveur et la base de test sont ceux du harness SQLite local; les appels API ne sortent pas de ce serveur.

Vérification produit finale : `cargo test --locked --offline --all-targets -- --test-threads=1` passe 552 réussis, 0 échec, 76 ignorés, 34 groupes (`all-targets.log`). `cargo fmt --all -- --check`, `git diff --check` et la garde de licence passent (`rustfmt-check.log`, `diff-check.log`, `license-guard.log`). Le readback de `cargo metadata` reste `license=null`, `license_file=null`, `publish=[]`; aucun choix de licence n’est fait.

## Interprétation et limites

Cette exécution confirme une frontière d’autorisation utile pour deux routes de protection avec ce rôle, ce dépôt et cette branche. Elle ne teste pas un merge dans ce même run, tous les rôles, toutes les routes ou tous les writers; elle utilise le checkout Forgejo scratch, qui est globalement modifié, et non un binaire officiel intact. Les deux fichiers de route utilisés sont toutefois inchangés depuis les blobs officiels indiqués. La conclusion ne neutralise pas POC-076 : ce même rôle peut toujours renommer une branche protégée via `PATCH /branches/{branch}`; merge whitelist n’est pas une autorité de mutation exclusive.

Aucun code Forgejo n’est adopté/copier dans le produit. Aucun coût, service distant, production ou secret utilisateur. Le readback de `cargo metadata` dans `cargo-metadata.json` garde `license=null`, `license_file=null`, `publish=[]`; la garde confirme `no_license_selected` et `redistribution_rights=not_assessed`. Aucun choix de licence n’est fait.
