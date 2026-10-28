# CONTINUATION-163 — readback canonique live local d’un commentaire de PR

Date : 26 octobre 2026. Worktree `/home/pc6/Documents/Codex/2026-10-21-mission-construire-une-forge-con-ue/.runtime/webhook-005-workspace`, HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`; le worktree était déjà fortement modifié. Aucun commit ni patch Forgejo n’a été fait.

## Question et résultat

Le test CONT-161 lisait un commentaire de PR via une fixture HTTP, mais les parcours d’instance live antérieurs ne vérifiaient que l’objet PR parent. Le runner a été renforcé pour exiger `is_pull=true`, effectuer une deuxième lecture API directe du commentaire, puis comparer le résultat du CLI à cette lecture séparée : ID, corps, date, `pull_request_url`, URL HTML avec ancre et label `untrusted_user_content`. Le résultat externe reste `resource.kind=pull_request`; le corps du webhook n’est pas traité comme état canonique.

Forgejo local a émis un vrai webhook `issue_comment/created` pour la PR #2. Le receiver l’a accepté; le CLI a lu deux ressources (`upstream_read_attempts=2`) avec le lecteur non-admin. Les valeurs exactes du readback CLI et d’un GET direct supplémentaire via l’API de l’instance concordent : commentaire ID `4`, `pull_request_url=http://127.0.0.1:3130/forge-admin/agent-forge-core/pulls/2`, `html_url=http://127.0.0.1:3130/forge-admin/agent-forge-core/pulls/2#issuecomment-4`, et même corps/horodatage. `body_trust` est `untrusted_user_content`. La PR est ensuite fermée sans merge. Le verdict du runner est `passed`.

## Vérifications fraîches

- TDD : le nouveau test de validation a d’abord échoué deux fois, car le validateur live dédié n’existait pas. Après implémentation, les tests ciblés du validateur et du parseur passent 20/20.
- `python3 tests/test_forgejo_emitted_webhook_e2e.py` : 114/114 réussis; sortie complète dans `python-e2e-suite.log`.
- `python3 -m py_compile scripts/run_forgejo_live_webhook_reconcile_emitted.py tests/test_forgejo_emitted_webhook_e2e.py` : succès, voir `pycompile.log`.
- Vérification ciblée des espaces finaux : six fichiers de code/documentation, aucun espace final (`targeted-whitespace.log`).
- `python3 scripts/verify_cargo_license_state.py` : vérifié, licence non sélectionnée et droits de redistribution non évalués (`cargo-license-check.log`).
- Le runner a construit `forgejo-webhook-local` et `forgejo-reconcile-local` via Cargo `--offline --locked` sous Bubblewrap avec `--unshare-net --unshare-pid`; journal et hachages dans `cargo-build.log` et `provenance.json`.
- Forgejo rapporte `17.0.0`; binaire SHA-256 `e5ec437cd6357e8d1b4580cb47ab1daa58a32b33971a63db63446a535d33adb7`. Le commit source exact de ce binaire n’est pas établi par `--version` ou son hash; l’audit du dépôt officiel v17.0.0 est documenté séparément.
- Nettoyage : le runner confirme les deux processus terminés, aucun listener TCP attendu, hook désactivé et relu. L’instance était un nouveau dossier `/tmp/forgejo-pr-comment-canonical-live-163`.

Commandes principales et contexte : `run-command.txt`, `toolchain-and-head.txt`. Les hash des sources réellement exécutées sont comparés au reçu de provenance dans `source-sha256.txt`.

## Portée et limites

Une seule instance locale jetable (`n=1`); Forgejo, receiver et CLI sont exécutés par le même utilisateur hôte. Le token de lecture est distinct du token propriétaire d’écriture, et le résultat CLI correspond au readback par le propriétaire; cela ne prouve pas une isolation de processus contre un processus hostile de même UID. Les deux GET du réconciliateur sont séquentiels et non atomiques. Aucun LLM, AgentOS, RAG ou mémoire n’a été utilisé; il n’y a pas d’injection exécutée, de comparaison avec CI conventionnelle ou de mesure de coût/productivité. Le parcours `deleted` d’un commentaire de PR sur instance live reste à démontrer. Pas de modification de production, service partagé ou source Forgejo upstream.

## Suites

1. Ajouter un scénario local de suppression de commentaire de PR : vrai webhook `deleted`, lecture parent réussie, route commentaire `404`, état `absent`; garder une création périmée avec `404` inconclusive.
2. Reprendre ensuite un autre critère ouvert de la matrice, sans rejouer la création déjà prouvée.
3. Comparaison contrôlée avec agent/CI conventionnels demeure suspendue à un plafond financier numérique fiable; aucun appel payant n’a été lancé ici.

## Manifests

Le manifeste local vérifie 30 fichiers de preuve. Les préfixes des index worktree et projet sont conservés; l’index parent lie l’index courant du worktree. Vérifications : `manifest-check.txt` et `global-manifest-check.txt`.
