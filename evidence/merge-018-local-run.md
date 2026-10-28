# MERGE-018 — whitelist de fusion mono-acteur, fail-closed

Date : 23 octobre 2026. Périmètre : validateur de protection de branche du gate local et fixtures loopback; aucun code Forgejo, instance de production ou service externe n’a été modifié.

## Constat et décision

validate_local_test_branch_protection exigeait que le login du gate figure dans merge_whitelist_usernames, mais acceptait d’autres utilisateurs et ignorait merge_whitelist_teams. Une règle pouvait donc laisser une seconde identité fusionner hors du gate alors que le résultat déclarait external_writer_exclusion: not_provided_by_cooperative_lock.

Dans Forgejo v17.0.0, épinglé pour l’audit au commit 0d791421f85ca13a63b665053d81ea765fbbc571, la structure API expose distinctement merge_whitelist_usernames et merge_whitelist_teams (modules/structs/repo_branch.go:33–35); l’API traduit les deux listes en identifiants d’utilisateur/équipe (routers/api/v1/repo/branch.go:620–657). Le readback local réel MERGE-016 (merge-016-live/branch-protection-readback.json) montre la forme attendue : un seul login forge-admin et une liste d’équipes vide.

Le chemin API de fusion v17 passe par CheckPullMergeable, qui appelle IsUserAllowedToMerge; avec une protection applicable, cette fonction s’appuie sur IsUserMergeWhitelisted. Quand la whitelist de fusion est activée, Forgejo teste les IDs d’utilisateurs et l’appartenance aux équipes autorisées, puis refuse si aucun ne correspond. Le routeur rend ce refus HTTP 405. Cela concorde avec le test live antérieur [non-whitelisted-merge-refusal.json](merge-016-live/non-whitelisted-merge-refusal.json) : « User not allowed to merge PR », PR toujours ouverte. Sources : [contrôle du merge](https://codeberg.org/forgejo/forgejo/src/commit/0d791421f85ca13a63b665053d81ea765fbbc571/services/pull/check.go), [résolution de l’identité](https://codeberg.org/forgejo/forgejo/src/commit/0d791421f85ca13a63b665053d81ea765fbbc571/services/pull/merge.go), [whitelist users/teams](https://codeberg.org/forgejo/forgejo/src/commit/0d791421f85ca13a63b665053d81ea765fbbc571/models/git/protected_branch.go), [réponse API](https://codeberg.org/forgejo/forgejo/src/commit/0d791421f85ca13a63b665053d81ea765fbbc571/routers/api/v1/repo/pull.go).

Le gate local refuse désormais une règle si la liste des logins n’est pas exactement [login du gate], si une équipe est autorisée, ou si le readback des équipes est absent/invalide. Cela valide la configuration observée avant le POST; cela ne rend pas Forgejo lui-même mono-autorité.

La décision canonique reste l’intégration de Forgejo par interfaces sous contrôle explicite des writers; [architecture.md](../docs/architecture.md) précise maintenant que le gate est seulement un pilote local et que l’absence d’exclusion externe reste ouverte. Aucun patch Forgejo n’est adopté.

Sources primaires : [structure API v17.0.0](https://codeberg.org/forgejo/forgejo/src/commit/0d791421f85ca13a63b665053d81ea765fbbc571/modules/structs/repo_branch.go), [création et mise à jour des protections](https://codeberg.org/forgejo/forgejo/src/commit/0d791421f85ca13a63b665053d81ea765fbbc571/routers/api/v1/repo/branch.go). Revue de la limite TOCTOU et de l’absence de CAS de base : [audit de l’API de merge](forgejo-merge-api-base-precondition-audit.md) et [MERGE-011](merge-011-base-freshness-race-local-run.md).

## TDD et résultat

Avant le changement de production, les deux nouveaux tests ciblant utilisateur supplémentaire et équipe supplémentaire ont échoué comme attendu : le validateur retournait Ok(()) dans les deux cas. Après le changement, les refus passent; un test supplémentaire refuse un readback sans champ merge_whitelist_teams.

Un test de parcours passe ensuite par le gate Rust, le client HTTP loopback et sa fixture : avec second-merger dans la whitelist, il observe le GET de la protection, retourne verdict=rejected, merge_attempted=false et n’émet aucun POST /pulls/23/merge. Les tests de profils par défaut restent valides.

Premier essai de compilation du test de parcours : erreur de test E0600, l’assertion appliquait ! à une serde_json::Value; elle a été remplacée par une lecture as_bool. Le premier rustfmt --check a aussi signalé deux lignes du nouveau code; rustfmt a corrigé ces écarts avant les tests finaux. Ce sont des erreurs d’écriture du test, pas des échecs du code du gate.

Vérifications fraîches sur ce snapshot :

- cargo test --offline --locked --all-targets -- --test-threads=4 : 32 groupes, 365 réussis, 0 échec, 14 ignorés; sortie dans [le journal complet](merge-018-all-targets.log).
- cargo clippy --offline --locked --all-targets -- -A clippy::too_many_arguments -A clippy::bool_assert_comparison -A clippy::manual_div_ceil : exit 0 avec ces trois allowances explicites; [journal](merge-018-clippy.log).
- rustfmt --edition 2024 --config skip_children=true --check src/bin/forgejo-reconcile-local.rs et git diff --check -- src/bin/forgejo-reconcile-local.rs : exit 0; [journaux](merge-018-rustfmt.log), [diff-check](merge-018-diff-check.log).
- cargo build --offline --locked --release --bin forgejo-reconcile-local : exit 0; [journal](merge-018-release-build.log).
- Une revue indépendante en lecture seule du validateur n’a trouvé ni défaut dans les refus ciblés ni incompatibilité avec les readbacks v17 examinés. Elle confirme que les tests prouvent la configuration contrôlée du gate, pas son exclusivité côté serveur. Les 14 tests ignorés par all-targets incluent des processus qui doivent être lancés par leurs harnais d’isolation dédiés.

## Limites et suite

Ce changement ne ferme pas la course où main avance après le dernier GET : l’endpoint public n’accepte pas de précondition de base atomique. BaseBranchMergeLock reste consultatif et ne bloque pas les clients Forgejo externes. Le code v17 et la preuve live couvrent la fusion via le endpoint PR protégé; ils ne démontrent pas l’exclusion d’un administrateur qui modifie la politique, d’un accès direct à la base/hôte, d’un writer sur un autre hôte ou de toutes les voies de mise à jour Git. La capacité du binaire conserve prevents_external_forgejo_writers=false.

Le test nouveau utilise une fixture HTTP loopback, pas une instance Forgejo; aucun droit de fusion réel n’a été reconfiguré pour ce changement. Le refus non-whitelisted sur Forgejo live est une preuve séparée du lot MERGE-016. Aucun LLM, AgentOS, Forge Code, RAG ou service mémoire n’a participé. La prochaine étape est de décider entre une contrainte d’exploitation de gate unique explicitement prouvée et une précondition serveur Forgejo, en chiffrant la maintenance upstream avant tout fork; puis exercer un refus de readback non exclusif sur une instance Forgejo jetable.

Le worktree reste préexistant et fortement modifié, HEAD f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0; aucun commit ou push n’a été créé.

Les empreintes des sources modifiées, du rapport et des journaux se trouvent dans [le manifeste SHA-256](merge-018-sha256.txt); sha256sum -c a validé chaque entrée.
