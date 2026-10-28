# CONTINUATION-197 — lier le scope CLI au scope de son stage

Date : 27 octobre 2026. Worktree : `webhook-005-workspace`, HEAD de base `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`, avec de nombreuses modifications préexistantes. Environnement : Python 3.12.3, Linux aarch64, systemd 255, cgroup v2 unifié. Aucun processus Claude en cours n’a été inspecté.

## Défaut reproduit

Après CONTINUATION-196, le nettoyage par groupe refusait les descendants restés dans le groupe, mais le test `setsid` montrait qu’un enfant pouvait rejoindre une session distincte. Le scope systemd de l’étape arrêtait bien le descendant lorsqu’il était encore dans son cgroup. En revanche, `run_bounded_cli` créait un second scope indépendant. Un test processus a lancé un CLI synthétique, attendu son reçu PID/cgroup, puis envoyé `SIGKILL` au processus stage. Avant correction, le runner hôte retournait alors que le CLI détaché était encore actif (`/proc/<pid>/stat`, état `S`). L’échec rouge complet est conservé dans `tdd-red-parent-sigkill.log`.

## Correction

`current_attested_process_scope()` identifie le scope user-systemd courant après lecture du cgroup par `/proc`, vérification du `ControlGroup`, de `KillMode=control-group`, de l’état actif et de l’appartenance du PID au `cgroup.procs`. Le lanceur CLI ne démarre qu’à partir de ce scope attesté. Il crée son propre scope avec `BindsTo=<unit parent>`; l’unité renvoyée par systemd doit refléter cette dépendance exacte avant que l’helper ne lise l’enveloppe et n’exécute la commande. Le démarrage refuse donc un scope non lisible ou un lien parent différent. Les processus du stage et du CLI gardent des cgroups distincts, mais l’arrêt du parent fait arrêter par systemd le scope enfant même si le processus stage a été tué par `SIGKILL` avant son `finally`.

Le test rouge/vert `test_parent_cleanup_removes_cli_scope_after_stage_is_killed` crée le vrai scope de stage puis le scope CLI lié, vérifie que le CLI a écrit son reçu, tue le stage à `SIGKILL`, attend le retour du runner hôte et vérifie que le descendant n’est plus actif. Le test `test_nested_scope_refuses_to_execute_after_its_parent_scope_stops` conserve un identifiant de parent valide puis arrête ce scope avant la création du scope enfant; systemd refuse le démarrage et le fichier marqueur n’est pas créé. Enfin, `test_systemd_runtime_limit_cleans_agent_tree_after_supervisor_sigkill` tue par `SIGKILL` le processus superviseur extérieur, sans lui laisser exécuter son cleanup Python. Le `RuntimeMaxSec` du scope arrête le stage et son CLI lié; le descendant est inactif et aucun scope de stage ne reste actif après environ six secondes pour un délai de stage de deux secondes. Le cas adjacent `test_agent_cli_uses_nested_scope_and_refuses_a_detached_survivor` confirme qu’un CLI normal fonctionne et que son propre enfant `setsid` est terminé avec statut inconclusif `125`. Une absence de systemd user bus, une attestation absente ou un échec de nettoyage reste refusée; aucun repli par simple liste positive réseau n’autorise le lancement.

## Vérification fraîche

`run_checks.py` capture les commandes et leurs journaux dans ce dossier :

- compilation Python : réussie;
- `test_mission_012_live_composed_e2e.py` : 12/12 réussis;
- `test_mission_012_mcp_supervisor.py` : 17/17 réussis;
- `test_mission_012_live_mcp_agent_e2e.py` : 10/10 réussis;
- `git diff --check` : code 0;
- readback systemd des scopes actifs `run-mission012-stage-*` après les suites : zéro scope actif.

Les arguments, codes de sortie et tailles des sorties sont dans `check-summary.json`; les sorties de test complètes sont séparées par commande. Aucun Cargo n’a été recompilé : ce lot ne change pas le code Rust. Aucun appel de modèle, Forgejo, réseau externe, secret, modification de production, publication ou dépense n’a été utilisé.

L’outil d’append a d’abord refusé l’absence du fichier local `SHA256SUMS`; son diagnostic est conservé dans `manifest-bootstrap-failure.log`. Après création d’un manifeste local vide avec préfixe nul, le vérificateur append-only a confirmé les préfixes worktree/projet ainsi que les empreintes latest-wins. Une première vérification `sha256sum -c` des sources fut lancée par erreur depuis le sous-dossier de preuve et n’a pas résolu les chemins relatifs; l’échec est conservé dans `source-hash-check-wrong-cwd.log`, puis le même contrôle a passé depuis la racine du worktree (`source-hash-check.log`). Les doublons historiques des index append-only ne doivent pas être vérifiés par `sha256sum -c` brut.

## Limites

Il s’agit d’un contrôle du harnais local sur une machine Linux/systemd 255 avec le gestionnaire systemd de l’utilisateur courant et cgroup v2. Les tests utilisent des processus synthétiques; ils n’exécutent ni LLM, ni AgentOS, ni tâche Forgejo ou PR. Cette dépendance d’un scope enfant à son parent n’isole pas plusieurs UID ou tenants, et ne protège pas contre un processus privilégié/compromis ayant l’autorité sur le gestionnaire systemd de l’utilisateur, une modification du cgroup par une autorité supérieure, ni une défaillance du noyau/systemd. Ce n’est pas une garantie de sécurité de production ou multi-hôte.

La mission reste incomplète : les parcours répétés avec un runtime agent autorisé et le benchmark apparié Forgejo + agent + CI ne sont toujours pas disponibles sans accès/coût nouveau; les droits de redistribution et l’autorité atomique de merge contre tous les writers restent ouverts.
