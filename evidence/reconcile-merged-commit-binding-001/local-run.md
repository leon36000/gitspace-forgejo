# WEBHOOK-RECONCILE-MERGED-COMMIT-BINDING-001

Date : 2026-10-25. Travail dans le worktree isolé, HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`, dont l’arbre était déjà fortement modifié. Les tests portent sur les sources de travail non commitées; les empreintes exactes figurent dans `source-sha256.txt`.

## Changement

La réconciliation d’une pull request Forgejo marquée `merged=true` exige maintenant `merge_commit_sha`, sous forme d’un SHA Git de 40 ou 64 caractères hexadécimaux. Un champ absent, nul ou mal formé rend la lecture inconclusive avec `InvalidCurrentStateField("merge_commit_sha")`. Pour une PR non fusionnée, le champ doit être absent ou nul. `ReconciledState` et le JSON `canonical_state` exposent le SHA. Le schéma de sortie versionné est passé à 3 pour une livraison et à 4 pour un lot; le résultat de découverte des capacités annonce ces versions.

Forgejo v17.0.0 a été examiné dans `/tmp/forgejo-v17-base-cas-poc`, basé au commit officiel `0d791421f85ca13a63b665053d81ea765fbbc571`. `modules/structs/pull.go` définit `MergedCommitID` comme `json:"merge_commit_sha"`; `services/convert/pull.go::ToAPIPullRequest` remplit ce champ lorsque `HasMerged`; `models/issues/pull.go::PullRequest.SetMerged` exige déjà un `MergedCommitID` non vide. Ces trois fichiers sont inchangés par rapport au commit de base dans le scratch. Aucun code Forgejo n’a été copié.

## Preuves rouge puis vert

- Le test contrat `merged_pull_request_without_merge_commit_sha_is_not_canonical` a d’abord échoué : le snapshot `merged=true` sans SHA était accepté comme état courant. Sortie rouge : `red.log`, exit `101`.
- Le test de processus `actual_reconcile_cli_process_refuses_merged_pull_request_without_merge_commit_sha` a reproduit le défaut dans le CLI réel avant correction. Le test échouait parce que le CLI retournait un succès pour une PR fusionnée sans commit de fusion; sortie `red-process.log`, exit `1`.
- Après correction, `cargo test --offline --locked --test webhook_reconcile_contract` passe 25 tests, dont refus du SHA absent et mal formé.
- Les deux tests CLI isolés passent sous le runner Bubblewrap avec réseau désactivé : refus du champ absent (`green-negative-process-final.log`, exit `0`) et propagation du SHA courant dans `canonical_state` (`green-positive-process-final.log`, exit `0`). Après le nettoyage d’une clé JSON dupliquée dans la fixture positive, ces deux scénarios ont été relancés. Le processus reçoit une fixture HTTP sur loopback; ce n’est pas une instance Forgejo.
- Suite finale après ce nettoyage, `cargo test --offline --locked --all-targets -- --test-threads=1` : 34 groupes, 480 tests réussis, 0 échec et 43 ignorés; exit `0` (`all-targets-final4.log`). Les cas CLI isolés ci-dessus ne sont pas comptés dans ces tests exécutés par le runner.
- `cargo build --release --locked --offline`, `cargo fmt --all -- --check` et `git diff --check` passent. Les sorties finales sont `release-build-final.log`, `rustfmt-final4.log` et `diff-check-final2.log`; tous les codes de retour sont conservés dans ce dossier.

Un essai du nouveau test mal formé a d’abord exposé un SHA `head.sha` de fixture invalide; cette erreur de fixture est gardée dans `malformed-merge-sha-fixture-error.log` et `.exit`. Après avoir repris le SHA valide d’un autre cas, le test ciblé passe dans `malformed-merge-sha-focused-green.log`.

## Versions et empreintes

Toolchain : Rust/Cargo 1.95.0, Bubblewrap 0.9.0, Linux ARM64. Les sources produit, les trois fichiers Forgejo utilisés comme références, et leur SHA-256 sont dans `source-sha256.txt`; les versions sont dans `tool-versions.txt`. Les commandes exécutées sont recensées dans `commands.txt`.

Le manifeste local couvre 54 artefacts et passe 54/54. Les entrées actuelles du manifeste global correspondant aux sources, au checkpoint et au dossier de preuve ont été vérifiées pour 64/64 chemins; l’historique global plus ancien n’a pas été réécrit ni réaudité dans ce lot.

## Portée et limites

Cette tranche confirme la présence et la syntaxe du SHA de merge dans un état API courant. Elle ne prouve pas que le commit contient le comportement attendu, ne lie pas cryptographiquement ce SHA au candidat testé, à sa base ou au tree effectivement fusionné, et ne remplace pas une vérification indépendante post-merge. Le parcours utilise des fixtures synthétiques et ne démontre ni un appel LLM, ni AgentOS, ni un gain contre Forgejo + agent + CI conventionnels. Il n’y a eu aucun appel externe payant, aucune publication et aucune opération de production.

Le changement de schéma peut demander une adaptation des consommateurs qui supposaient les versions 2/3. Les métadonnées Cargo restent `license=null`, `license_file=null`, `publish=[]`; aucun choix de licence ou droit de redistribution n’est inféré.
