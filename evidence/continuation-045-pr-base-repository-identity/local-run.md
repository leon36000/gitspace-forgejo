# CONTINUATION-045 — identité du dépôt de base d’une PR réconciliée

Date d’exécution : 26 octobre 2026. Worktree : `.runtime/webhook-005-workspace`, HEAD produit `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`.

## Écart et correction

La réponse API actuelle d’une PR contient `base.repo`. La validation comparait les champs d’identité fournis, mais acceptait `{}` puisque aucun champ ne contredisait le dépôt configuré. Le test rouge `pull_request_current_base_repository_is_bound_while_foreign_head_is_allowed` reproduit l’acceptation et attend `InvalidCurrentStateField("base.repo")`.

`src/webhook/reconcile.rs::validate_current_pull_request_base_repository` exige maintenant, si `base.repo` existe et n’est pas `null`, au moins un identifiant exploitable parmi `full_name`, `name` et `owner.login`; tous les champs d’identité fournis continuent à être validés contre le dépôt cible. `base.repo` absent ou `null` reste autorisé pour les réponses partielles. L’identité de `head.repo` n’est pas forcée au dépôt cible, afin de garder les forks valides.

## Source amont

Forgejo v17.0.0, commit `0d791421f85ca13a63b665053d81ea765fbbc571`. Dans `modules/structs/pull.go`, `PullRequest.Base` et `.Head` sont des pointeurs `*PRBranchInfo` sérialisés sous `base` et `head`; `PRBranchInfo.Repository *Repository` est sérialisé sous `repo`. Vérification locale : `git diff --quiet HEAD -- modules/structs/pull.go` donne 0; SHA-256 du blob exact `1472f689d9e80da512cc8edafa5d647daafccdf060dfba0d4a74d3be5cfe06e3`. Le checkout scratch Forgejo est globalement modifié par des POC indépendants; aucun code amont n’a été copié ou modifié.

## Vérifications

- Rouge pré-correction : 0 réussi, 1 échec, 39 filtrés; le snapshot vide a été accepté à tort. Sortie conservée dans `unit-red.log`.
- Contrat de réconciliation : 40/40. La table couvre les identités étrangères, `base.repo` mal formé/vide, `owner` sans `login`, les métadonnées valides partielles (`name` ou `owner.login`), `null` et le champ absent. Une PR issue d’un fork reste valide.
- Découverte machine-readable : le test rouge de `capabilities_list_exact_merge_target_profiles` échouait faute de contrat; il passe 1/1 après l’ajout de `current_api_identity_contract`, qui décrit les champs d’identité reconnus, la règle d’objet présent/non nul, les réponses partielles et les forks `head`.
- Le test du processus CLI `actual_cli_discloses_versioned_local_test_capabilities` relit `forgejo-reconcile-local --capabilities` et vérifie l’objet JSON exact : 1/1.
- Processus isolé : le vrai receiver accepte une notification PR signée, l’événement durable est relu par le vrai `forgejo-reconcile-local`, qui appelle une fixture HTTP loopback au seul endpoint configuré. Le CLI refuse `base.repo: {}` par `inconclusive/invalid_current_state`, `details.field=base.repo`; exactement un GET, aucun header Authorization, aucun `canonical_state`, inbox inchangée byte-for-byte. Exécution explicite du test ignoré par défaut sous Bubblewrap `--unshare-net --unshare-pid` : 1/1.
- `cargo test --locked --offline --all-targets -- --test-threads=1`: 570 réussis, 0 échec, 82 ignorés, 34 groupes, relancé après le contrat `--capabilities`. Le scénario processus additionnel est lancé à part et reste ignoré dans la matrice générale, car le test cible le port fixe allowlisté.
- `cargo fmt --all -- --check`, `git diff --check`, metadata Cargo et garde de licence passent; les neuf tests du garde passent.

## Licence Cargo et limites

Le readback courant de `agent-forge-core 0.1.0` reste `license=null`, `license_file=null`, `publish=[]`, `decision=no_license_selected`, `redistribution_rights=not_assessed`, `spdx_package_license_declared=NOASSERTION`. Cette vérification conserve l’absence de choix; elle n’attribue aucun droit.

La fixture ne prouve pas le comportement d’une instance Forgejo live ni l’exécution upstream Go (`go` est absent du PATH). Pas d’intégration LLM/AgentOS, de merge, de mesure comparative, de service externe, de coût nouveau, de publication ou de production dans ce lot. Le test vérifie un refus de contrat dans une fixture; il ne prouve pas la sûreté de toutes les réponses Forgejo.

Commandes, logs bruts, versions, source et hashes sont joints à ce dossier. Les manifestes locaux/produit/parent sont générés par `update_manifests.py` et leurs préfixes append-only sont contrôlés.
