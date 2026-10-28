# CONTINUATION-221 — parcours MISSION-012 sous scope hôte attesté

Date de l’exécution : 2026-10-27. Environnement local isolé; aucune instance de production, publication ou service externe.

## Résultat

Un parcours composé complet s’est terminé avec `verdict=passed` dans `supervised-private-netns-observation.json`. Le runner crée le scope transitoire systemd depuis l’hôte avant Bubblewrap. Dans le namespace utilisateur/réseau privé, le composeur réutilise uniquement ce scope parent attesté et compare l’ensemble exact des PID actifs avant et après chaque stage. Une différence refuse le stage; le nettoyage externe arrête les membres résiduels. L’agent de test, le gate et Forgejo restent des processus distincts selon leur rôle.

Un premier run conservé séparément s’est arrêté sur une erreur actionnable du harness : `FAIL-CLOSED: name 'shutil' is not defined`. Il n’a pas autorisé de merge; le scope a été nettoyé (`clean`). L’import manquant a été ajouté. Le run suivant est le résultat `221b` décrit ici. Le test ciblé crée également un descendant `setsid` qui survit au groupe initial : la comparaison de PID refuse le succès et le scope hôte le termine.

## Parcours observé

- Admission par livraison webhook versionnée, allocation `mission012-continuation-221b`; exigences protégées liées par SHA-256 `3a3c444075c93a3bcfb2bec17d1a2e74ec0051effc2994e6b02ac095eb0dd431`.
- Forgejo v17 local reçoit l’issue #1 et la PR #2; contexte courant fourni via `local-test-adapter`.
- La fixture démarre avec un échec d’acceptation : `values_outside_the_range_clamp_to_the_nearest_boundary` échoue. L’adaptateur modifie seulement `src/lib.rs`; les tests candidats passent 2/2.
- Une vérification indépendante identifiée `mission-012-rust-isolated-cargo-acceptance-v2` relie le commit candidat `f32cff1ca73d4866bfdbd6baf83d9a94bcccc007`, la base `21d7c60f7f4adc5ca26f5f8e76d9208d2b13a8f1`, la version des exigences et l’empreinte de politique. Le gate fonctionne pendant que le processus MCP hôte est vivant. Une fusion sans revue est refusée (HTTP 405).
- Le gate reçoit le credential de merge par FD uniquement à son exécution. Les probes du processus candidat vers les routes Forgejo API, Web et `git-receive-pack` sont bloquées; les canaris de credential/runtime restent invisibles. Le superviseur et Forgejo partagent leur namespace réseau privé, distinct du namespace hôte; le test depuis l’hôte vers le port Forgejo reçoit `ECONNREFUSED`, et le `setns` non privilégié est refusé.
- La PR autorisée est fusionnée après revalidation : commit `ca2e7e49de0ecd1d5b0500915fa23d1291811e61`. Le readback Forgejo confirme que `main` pointe exactement sur ce commit. L’acceptation post-fusion exécute `cargo test --offline --locked --test acceptance` sur le commit lu après fusion : 2 réussis, 0 échec.
- Le scope hôte s’est nettoyé (`clean`), sortie bornée, aucun timeout. La vérification du manifeste des reçus bruts confirme 18/18 fichiers (`SHA256SUMS` SHA-256 `a915de6dc6f73ce8fc6e2eb4c6233c3c38b22146115669e830849212e82387a0`).

Les reçus détaillés sont dans [l’observation supervisée](../mission-012-live-agent-webhook-continuation-221b-scope/supervised-private-netns-observation.json), notamment les 16 contrôles, et dans les autres fichiers du même dossier : admission, issue/PR Forgejo, refus avant revue, vérification indépendante, vérification pendant la vie du MCP, autorité Rust de merge, revalidation et readback post-fusion. Le test et les commandes sont résumés dans `commands.txt`; versions et hashes figurent dans `tool-versions.txt` et `source-sha256.txt`.

## Réutilisation et frontière de confiance

Le code ne baisse pas les exigences de `prepare_process_scope()` dans une user namespace. L’expérimentation préalable montre que `systemd-run` y apparaît comme UID 65534 et le refuse; le parcours principal atteste donc le cgroup depuis l’hôte puis l’inspecte depuis la sandbox. Le nouveau mode est opt-in et limité à `local-test-adapter`. Il accepte uniquement un scope parent dont l’identité et le cgroup sont attestés et dont le PID set demeure stable autour de chaque commande; l’exit process ne suffit pas à annoncer le succès. L’enveloppe externe garantit le nettoyage même si un enfant s’est détaché par `setsid`.

## Preuves fraîches dans ce lot

- `test_mission_012_live_composed_e2e.py` : 17/17 tests passés, dont refus sans scope, refus après changement d’ensemble de PID et terminaison d’un enfant détaché.
- `test_mission_012_live_mcp_agent_e2e.py` : 13/13 tests passés, dont refus sans scope attesté et après modification de l’ensemble des PID.
- `py_compile` sur les quatre scripts du runner : succès.
- `git diff --check` : succès.
- `verify_cargo_license_state.py` : `license=null`, `license_file=null`, `publish=[]`, `no_license_selected`, droits `not_assessed`, `NOASSERTION`.
- `sha256sum -c SHA256SUMS` dans les reçus `221b` : 18/18 OK.

Les sorties correspondantes sont archivées dans les fichiers `.log` de ce dossier; les empreintes courantes des sources et documents sont dans `source-sha256.txt`.

## Limites matérielles

C’est un parcours déterministe local `n=1`, avec `local-test-adapter`; aucun LLM, AgentOS, Forge Code ou RAG n’a été invoqué. Les champs du run indiquent `model_invoked_during_run=false`, `llm_invoked=false` et coût déclaré 0 USD; aucune facturation de fournisseur n’a été engagée. Ce run ne mesure pas la valeur comparée à Forgejo + agent + CI conventionnels et n’établit aucun gain.

La politique indépendante du gate ne prouve toujours pas que tous les writers ont été exclus : `external_writer_exclusion=trusted_supervisor_policy_not_independently_verified`. Les sondes API/Web/receive-pack couvrent uniquement ces chemins épinglés et cette configuration. Le modèle exclut un acteur root/privilégié, un noyau compromis, une évasion de namespace et les autres OS/runtimes. Les limites de sortie et délais sont bornés, mais aucun plafond CPU/mémoire/tâches général de ce run n’est démontré. Le gate s’est observé pendant que le MCP est vivant; le MCP n’est pas maintenu jusqu’à la fusion. Le binaire Forgejo n’a pas de commit de build établi. Le worktree produit est très modifié et aucun commit n’a été créé.

## Suite

Tester annulation et reprise après interruption dans le mode scope-parent attesté, en prouvant l’invalidation des preuves et l’absence de processus résiduel. Puis continuer le benchmark apparié (si un agent autorisé et sans dépense nouvelle est disponible), la matrice des refus et les droits/licences. Aucun nouvel appel payant ou déploiement n’est autorisé par cette preuve.
