# MERGE-020 — fusionneur non-admin, lecteur de règle séparé

Date : 24 octobre 2026. Exécution locale sur le dépôt de travail `agent-forge-core`, sans publication ni modification de production. Le snapshot de travail est basé sur HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`; aucun commit n’a été créé.

## Références épinglées

- Forgejo source : dépôt officiel `https://codeberg.org/forgejo/forgejo.git`, tag `v17.0.0`, commit `0d791421f85ca13a63b665053d81ea765fbbc571`. Le clone source ciblé est `/tmp/forgejo-v17-merge-audit`; le binaire de l’E2E n’est pas relié de manière vérifiable à ce clone source.
- Forgejo binaire local : `/tmp/forgejo-17.0.0`, version annoncée `17.0.0+gitea-1.22.0`, SHA-256 `e5ec437cd6357e8d1b4580cb47ab1daa58a32b33971a63db63446a535d33adb7`.
- Instance : base SQLite, dépôts, configuration et logs isolés dans `/tmp/forgejo-merge-020-nonadmin-003`; port loopback 3130. Le processus s’est arrêté dans le `finally` du pilote et le port était libre après le run. Les fichiers conservés ici ne contiennent ni base de données, ni token brut, ni clé HMAC.
- Code concerné : [gate Rust](../../src/bin/forgejo-reconcile-local.rs) — option et lecture du second FD autour des lignes 2626–2690, politique de requête `MergePolicyReadOnly` autour de 918–929, contrôle de `unprotected_file_patterns` autour de 1985–2030, capacité autour de 3488–3510, tests autour de 5820 et 7790; [pilote Forgejo live](../../scripts/run_forgejo_live_pr_merge_e2e.py) — comptes/roles autour de 1330–1460, règle et test de portée autour de 1690–1805, pipes et appels du gate autour de 1090–1160 et 1810–2090.

## Base du choix d’intégration Forgejo

Le code v17 permet une whitelist de fusion limitée à un utilisateur : `models/git/protected_branch.go:153–173` et `services/pull/merge.go:476–491`. L’API de merge PR passe par `CheckPullMergeable` et renvoie 405 à un acteur refusé : `routers/api/v1/repo/pull.go:947–953`. Le GET et les mutations de `/branch_protections` sont groupés derrière `reqToken(), reqAdmin()` (`routers/api/v1/api.go:902–910`); `reqAdmin` couvre un owner, un repo-admin ou un site-admin (`api.go:353–359`). Avec le token étroit `write` de `forge-merger`, le premier essai réel du gate échouait donc en `upstream_forbidden` au GET de cette règle, avant toute tentative de POST. La lecture du source identifie le blocage; les deux premiers runs confirment le comportement live.

Le pre-receive v17 laisse passer les pushes directs qui ne touchent que les `unprotected_file_patterns` lorsque le push ordinaire est bloqué (`routers/private/hook_pre_receive.go:381–418`). Le gate local exige à présent la présence d’une chaîne vide dans le readback. Ceci impose une condition au gate local; ce test n’exerce pas un push direct Forgejo.

## Exécution reproductible

Le répertoire d’état doit être neuf et enfant direct de `/tmp` :

```sh
python3 scripts/run_forgejo_live_pr_merge_e2e.py \
  --forgejo-binary /tmp/forgejo-17.0.0 \
  --repo-root "$PWD" \
  --state-dir /tmp/forgejo-merge-020-nonadmin-003 \
  --non-admin-merger
```

Le pilote bâtit le client/gate local, démarre une nouvelle instance, crée les comptes synthétiques, met en place une PR réelle, teste son candidat, puis arrête Forgejo. Il n’invoque pas Claude, un LLM, AgentOS, RAG, une mémoire ou Forge Code. L’agent est le `deterministic local test adapter`; la modification logicielle est réelle et exécutable mais n’est pas produite par un LLM dans ce run.

Réexécution des vérifications du contrat :

```sh
cargo fmt --check
python3 -m py_compile scripts/run_forgejo_live_pr_merge_e2e.py
cargo test --offline --locked --bin forgejo-reconcile-local
cargo test --offline --locked --test forgejo_reconcile_capabilities_e2e
```

Résultats finaux : `cargo test --offline --locked --all-targets -- --test-threads=4` — 32 groupes, 371 réussis, 0 échec, 14 ignorés (`cargo-all-targets.log`); Clippy all-targets avec les trois allowances historiques (`too_many_arguments`, `bool_assert_comparison`, `manual_div_ceil`) passe (`cargo-clippy.log`); build release all-targets passe (`cargo-release-build.log`). `cargo fmt --check`, `python3 -m py_compile scripts/run_forgejo_live_pr_merge_e2e.py` et `git diff --check` passent. Le run ciblé a aussi exécuté 84 tests du binaire et 2 tests de capacités avec 0 échec. Le test `merge_gate_uses_separate_policy_reader_and_non_admin_merge_credentials` atteste dans une fixture HTTP que tous les GET de règle portent le jeton lecteur et que `POST /merge` porte le jeton acteur. Le test `merge_policy_reader_allows_only_the_exact_branch_protection_get` refuse les autres chemins, les méthodes d’écriture et les GET avec corps. Les tests de validateur refusent un champ `unprotected_file_patterns` non vide ou absent.

## Résultats Forgejo observés

- `forge-merger` est `is_admin=false`, permission de dépôt `write`, et seul login de `merge_whitelist_usernames`. Le token d’acteur a les scopes `read:user,read:issue,write:repository`.
- `forge-policy-reader` est `is_admin=false`, permission de dépôt `admin`, token `read:user,read:repository`. Il lit la règle exacte `main`; `DELETE` d’un nom inexistant est refusé HTTP 403 par le scope sans changer la règle. Le token n’est transmis qu’au canal privé du gate dans le run; l’API Rust le limite encore au GET exact.
- La protection lue à la création et à la fin exige une approbation, liste `forge-reviewer` pour l’approbation, liste `forge-merger` pour la fusion, `apply_to_admins=true`, bloque les poussées ordinaires et expose `unprotected_file_patterns=""`.
- Avant correction, cinq cas d’acceptation échouent et quatre passent. L’adaptateur applique `value.clamp(min, max)`; les neuf cas passent. Un binaire candidat qui ne fait que retourner succès sans résultat est refusé (0/9). Une preuve stale est refusée.
- L’API Forgejo renvoie 405 pour le fusionneur sans approbation (`Does not have enough approvals`), le reviewer non whitelisté et le site-admin hors whitelist (`User not allowed to merge PR`). Le Rust gate refuse aussi l’acteur approuvé mais non whitelisté et n’envoie pas son POST.
- Le POST final du gate en tant que `forge-merger` retourne HTTP 200. Le readback final observe la PR fermée/fusionnée, le commit de fusion à la tête de `main`, l’issue fermée, et le contenu source de la PR identique aux octets testés. Voir `result.json`, `branch-protection-readback.json`, `policy-reader-role-readback.json`, `red-verifier.log`, `green-verifier.log`, refus et readbacks dans ce dossier.

## Portée et risques non clos

Forgejo stock vérifie la whitelist sur la voie PR API/UI mais n’exige pas le reçu Rust. L’acteur whitelisté peut donc appeler directement Forgejo s’il obtient son token; le run ne prouve pas l’isolation de processus d’un futur service. Le compte lecteur détient le rôle repo-admin malgré son token lecture; un détenteur de son mot de passe ou un administrateur de site peut changer la règle. L’admin site peut modifier la whitelist avant un merge. Forgejo v17 n’accepte pas une base attendue comme CAS sur l’API merge : le verrou local est consultatif et la course d’un writer extérieur reste ouverte. L’empreinte des artefacts identifie leur provenance/intégrité, pas la correction du code. Un succès live, un seul run, n’établit ni taux de réussite général, ni baisse d’interventions, de tokens, de durée ou de coûts.

Les readbacks, logs du vérifieur, résultat E2E et journaux de build/tests sélectionnés sont expurgés et listés dans `SHA256SUMS`. Le log Forgejo complet, la base SQLite, `verifier.key`, les mots de passe aléatoires et tokens ne sont pas ajoutés au dépôt.

## Complément après revue indépendante : rôle effectif du fusionneur

La revue a noté que le pilote accordait `write` au fusionneur mais ne conservait pas le readback Forgejo de ses permissions effectives. Le pilote exige maintenant `permissions.admin == false` et `permissions.push == true` dans la réponse `GET /repos/{owner}/{repo}` authentifiée par le jeton de `forge-merger`, et écrit `merge-account-role-readback.json`.

Rejeu complet le 24 octobre dans un état neuf `/tmp/forgejo-merge-020-nonadmin-004`, avec la même commande ci-dessus et le harnais mis à jour. La réponse Forgejo confirme `is_admin=false`, `repo_permission_admin=false`, `repo_permission_push=true`; le parcours aboutit aussi à 9/9 cas d’acceptation verts et au merge HTTP 200. Le résumé non secret du run est `followup-run-result.json`; le hash du pilote, celui du binaire Forgejo et l’identifiant d’état sont consignés dans `followup-run-provenance.json`. Aucun secret, base, journal serveur complet ou artefact binaire n’a été ajouté. Le manifeste couvre ces trois fichiers supplémentaires; il relie les octets conservés, pas la vérité runtime ni la correction.
