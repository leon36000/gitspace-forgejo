# CONTINUATION-445 — identité du dépôt base au readback post-fusion

Date : 28 octobre 2026. Worktree local : `.runtime/webhook-005-workspace`, base observée `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`; dépôt déjà fortement modifié, aucun commit ou reset effectué.

## Constat et changement

`merge_readback_is_confirmed` vérifiait séparément le readback de fusion et n’exigeait que `base.repo.full_name`. La réponse pouvait donc conserver `forge-admin/mission-012-clamp` dans ce champ tout en renvoyant un `base.repo.owner.login` étranger. Le test unitaire `merge_readback_refuses_divergent_commit_and_issue_state` a reproduit ce cas avant correction; son stdout rouge n’a pas été archivé dans un fichier.

Le gate compare désormais ensemble `base.repo.full_name`, `base.repo.name` et `base.repo.owner.login` à l’owner et au dépôt de la cible. Tout champ absent, mal typé ou divergent rend le readback non confirmé. Le schéma `--capabilities` expose les trois chemins requis dans `postmerge_pull_request_identity` et exige qu’ils correspondent au profil de cible sélectionné.

Le test processus `merged_readback_with_contradictory_base_owner_is_inconclusive` exerce le vrai binaire CLI dans le runner Bubblewrap réseau/PID isolé. La fixture applique le POST, puis renvoie un readback où `full_name` et `name` correspondent mais où `owner.login` est étranger. Le résultat observé est `inconclusive / merge_outcome_not_confirmed`, un événement durable `mission_merge_attempt_started_v1` et zéro `mission_merge_attempt_resolved_v1`. Ce scénario a passé 1/1 avant le run all-targets; sa sortie brute n’a pas été conservée séparément et il n’a pas été relancé pour éviter de répéter le même scénario.

## Vérifications

- `cargo test --locked --offline --bin forgejo-reconcile-local -- --test-threads=1` : **167 réussis, 0 échec**; sortie archivée dans `unit-tests.log`.
- `cargo test --locked --offline --all-targets -- --test-threads=1` : **39 groupes, 786 réussis, 0 échec, 207 ignorés**; sortie archivée dans `all-targets.log`. Les tests ignorés ne sont pas comptés comme exécutés.
- `cargo fmt --all -- --check`, `git diff --check`, `python3 scripts/verify_cargo_license_state.py` : passent.
- Le test processus Bubblewrap ci-dessus : **1/1**, résultat déjà observé avant la suite complète; commande consignée dans `commands.txt`.

## Sources, environnement et limites

Le contrat JSON attendu correspond à Forgejo officiel v17.0.0 au commit `0d791421f85ca13a63b665053d81ea765fbbc571`: `PRBranchInfo.Repository` référence `Repository`; `Repository` sérialise `Owner`, `Name`, `FullName`; `User.UserName` est sérialisé comme `login`; `ToAPIPullRequest` remplit le dépôt base par `ToRepo`. Les fichiers et symboles examinés sont précisés dans `upstream-source.txt`. Aucun code Forgejo n’a été copié ou modifié.

La preuve E2E utilise une API loopback synthétique, un seul scénario (`n=1`) et un POST que la fixture applique réellement. Elle vérifie que le client n’annonce pas une fusion confirmée quand son readback contredit l’identité de base; elle ne l’empêche pas, ne l’annule pas et ne fournit pas le CAS manquant de Forgejo. Elle ne prouve ni l’intégrité/authenticité du serveur, ni une fusion sur Forgejo natif, ni un parcours AgentOS/LLM, ni un gain comparatif. Les versions, l’état sale et les empreintes figurent dans `environment.txt`; les commandes et sorties sont les autres fichiers de ce dossier.

État Cargo relu : `license=null`, `license_file=null`, `publish=[]`, `no_license_selected`, droits de redistribution `not_assessed`, SPDX `NOASSERTION`. Aucun choix de licence n’a été ajouté.
