# CONTINUATION-250 — observation hôte du processus agent au gate

Date : 27 octobre 2026. Worktree `webhook-005-workspace`, HEAD de base `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`, fortement modifié. Aucun commit, publication, accès externe, production, credential réel ou appel de modèle.

## Défaut de preuve et correction

La revue du runner a trouvé que `run_task_at_barrier` appelait `poll()` sur le processus wrapper Python et enregistrait son résultat comme `child_process_alive_during_gate_read`. Le wrapper peut rester vivant tandis que son descendant sandboxé s’arrête; ce champ seul n’attestait donc pas l’identité ou la vie du processus agent.

Le runner trusted parcourt maintenant l’arbre descendant dans `/proc` quand l’adaptateur atteint la barrière, exige exactement un processus Python dont `argv` cible `/opt/mission_012_test_agent_adapter.py`, et lit son PID hôte, son start-time et ses namespaces réseau/PID. Il refait la lecture après l’opération du gate, exige le même PID/start-time/namespaces, puis compare les namespaces observés par l’hôte au rapport de l’enfant. Un processus hôte sans adaptateur sandboxé est refusé. L’observation est dans `mcp_barrier.sandbox_agent_process_observation`; elle ne dépend pas du compteur de connexions à un listener éphémère.

Le test positif utilise Bubblewrap et l’adaptateur réel; le test négatif passe un processus Python hôte au vérificateur et attend un refus. Le test positif initial a échoué car la fonction d’observation n’existait pas encore; cet échec de mise en place n’est pas présenté comme une ancienne acceptation de sécurité reproduite.

## Parcours live local

Commande : voir [`commands.txt`](commands.txt). Le runner a lancé un Forgejo jetable `17.0.0+gitea-1.22.0`, binaire SHA-256 `e5ec437cd6357e8d1b4580cb47ab1daa58a32b33971a63db63446a535d33adb7`; le commit exact qui a construit ce binaire est inconnu. Le processus sandboxé a essayé l’API PR, la Web UI et le POST Git `receive-pack` sur le listener réel `127.0.0.1:3130`; le namespace Bubblewrap `--unshare-net --unshare-pid` et les trois refus ont été observés pendant que le gate fonctionnait. La branche principale Forgejo répond au GET authentifié du gate en HTTP 200; le gate séparé reçoit HTTP 200 pour la fusion, puis relit le même commit de fusion comme tête de `main`. Aucun des trois probes du processus agent ne reçoit le credential de fusion.

Observation depuis le parent trusted, avant et après l’opération : même adaptateur PID hôte `3928816`, start-time `24733808`, namespace réseau `net:[4026536249]` contre l’hôte `net:[4026531833]`, namespace PID `pid:[4026536248]` contre l’hôte `pid:[4026531836]`. L’identité renvoyée par l’adaptateur concorde avec `/proc`. Le credential synthétique du gate reste en mémoire/FD privé, n’est ni dans l’environnement, la commande, la configuration MCP ou un fichier, ni hérité par l’enfant; le probe `/proc/<pid>/fd/<fd>` échoue dans son namespace PID.

L’acceptation initiale produit un échec actionnable (5 cas échoués, code 101); l’adaptateur corrige le fichier source via MCP et les tests protégés passent 9/9. Après fusion, les mêmes 9 tests passent sur le contenu dont le SHA correspond au candidat. Les reçus JSON et journaux sélectionnés sont copiés ici; l’état complet Forgejo temporaire reste sous `/tmp/forgejo-agent-cont250-state-001` et n’a pas été supprimé. Le port 3130 n’écoute plus après le run.

## Vérifications, portée et limites

- `tests.test_forgejo_live_pr_merge_e2e` : 17/17; inclut l’observation positive de la descendance sandboxée et le refus du processus hôte.
- `tests.test_mission_012_sandboxed_test_adapter` : 9/9.
- `py_compile` des trois fichiers Python concernés, audit d’espaces de fin sur les fichiers texte de ce lot et `git diff --check` final après mise à jour des documents : réussite.
- Parcours Forgejo local composé : `merged=true`, POST merge 200, readback de `main` égal au commit fusionné et post-merge acceptance 9/9. `supervisor_policy_independently_verified=false` et `external_writers_excluded=false` restent vrais.

C’est un run local unique (`n=1`) avec le `local-test-adapter` déterministe, sans LLM ni AgentOS. L’observation `/proc` et le refus réseau ne certifient pas Bubblewrap contre un noyau compromis, les processus privilégiés, un autre UID exécuté hors sandbox ou toutes les routes/protocoles. `receive-pack` est testé sans le credential de fusion. Le binaire Forgejo a une empreinte connue, mais son commit de build n’est pas établi. Aucun benchmark comparatif, coût/tokens réel ou gain n’est démontré; la suite Rust complète n’a pas été relancée.

La sortie brute des reçus copiés, les commandes, versions, empreintes et préfixes de manifestes sont conservés dans ce dossier; le helper append-only n’écrase aucun manifeste antérieur.
