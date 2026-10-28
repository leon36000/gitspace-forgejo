# CONTINUATION-280 — respecter `read_current_fresh` dans le cache de batch

Date : 27 octobre 2026  
Worktree : `.runtime/webhook-005-workspace`  
HEAD de base : `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0` (worktree fortement modifié, aucun commit créé).

## Lacune et changement

`ForgejoCurrentState::read_current_fresh` demande un readback qui contourne/invalide un éventuel cache de l’adaptateur. `CachedCurrentState` respectait cette instruction pour son propre cache en l’évincant, mais rappelait ensuite `source.read_current`. Un adaptateur avec un cache interne pouvait donc retourner la réponse parent périmée. Dans le cas d’un commentaire supprimé, un 404 du commentaire suivi d’une permission parent révoquée pouvait alors produire une projection « absent » qui n’était pas suffisamment confirmée.

Le test `batch_deleted_pr_comment_404_preserves_adapter_fresh_read_contract` simule cette frontière : `read_current` sert toujours l’ancien parent, tandis que le `read_current_fresh` de la source retourne `NotFound`. Avant le changement, le test échoue avec un ancien état `closed` rendu canonique. Le cache batch délègue maintenant l’appel à `source.read_current_fresh`; après le changement, le test donne le refus `Source(NotFound)` attendu.

## Vérification

Commande de reproduction rouge :

```text
cargo test --locked --offline --test webhook_reconcile_contract batch_deleted_pr_comment_404_preserves_adapter_fresh_read_contract -- --exact
```

Résultat rouge avant le changement : échec d’assertion attendu; résultat observé `Ok(ReconciledState { ... state: "closed", comment.state: "absent" })`, attendu `Err(Source(NotFound))`.

La commande finale dans `commands.txt` exécute les deux suites touchées et les contrôles de format, diff, licence et metadata. Résultat : **82 tests Rust réussis**, format et diff propres, garde de licence réussie et **12 tests de politique licence réussis**. Les sorties détaillées et le résumé Cargo sont archivés dans ce dossier.

## État de licence Cargo

Readback frais de `cargo metadata --locked --offline --no-deps` : package `agent-forge-core`, `license=null`, `license_file=null`, `publish=[]`; metadata de revue `no_license_selected`, droits `not_assessed`, marqueur d’export `NOASSERTION`. Aucun SPDX de licence n’est ajouté comme licence Cargo, aucun droit n’est inféré et la publication reste désactivée. Cela ne tranche pas les droits sur GitSpace, ses dépendances, ni la licence à choisir plus tard.

## Portée et limites

Le changement est dans le décorateur cache de l’interface de lecture Forgejo; il ne change ni le receiver, ni le format du journal, ni l’autorité de merge. Les tests utilisent des objets `JournalEvent` et un adaptateur synthétique. Le `read_current_fresh` de l’adaptateur HTTP courant hérite de la lecture directe par défaut; aucune instance Forgejo, aucun credential, aucun webhook émis par serveur, aucun LLM/AgentOS et aucun merge ne sont exercés ici. Cette tranche ne prouve pas une lecture atomique face à des modifications concurrentes côté serveur.

Toolchain exacte et hashes des sources courantes : `toolchain.txt`, `source-sha256.txt`. Les préfixes des manifestes append-only du worktree et du projet sont préservés par `append_manifests.py` et contrôlés par `verify_manifests.py`.
