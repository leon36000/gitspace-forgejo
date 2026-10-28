# CONTINUATION-196 — refuser le succès si un descendant du groupe survit

Date : 27 octobre 2026. Dépôt local : worktree `webhook-005-workspace`, base `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`, fortement modifié. Aucun processus Claude déjà lancé n’a été inspecté, contacté ou relancé.

## Lacune et reproduction

Les deux lanceurs attendaient le chef du groupe et ses flux de sortie, mais le processus principal pouvait créer un descendant qui fermait ses sorties puis quitter. Le lanceur d’étape retournait alors le code 0 avec un enfant encore actif; le lanceur CLI lançait un nettoyage de groupe dans son `finally`, mais ne reflétait pas ce descendant dans le statut de sortie. Un tel processus peut prolonger des actions après la réponse de l’agent.

Deux tests processus utilisent un vrai processus Python et un enfant `sleep` vivant, redirigent les sorties de l’enfant vers `/dev/null`, puis vérifient `/proc/<pid>/stat`. Les deux tests ont d’abord échoué sur le résultat observable attendu, code 0 au lieu du refus 125. Les sorties rouges sont gardées dans `tdd-red-stage.txt` et `tdd-red-cli.txt`.

## Changement

`scripts/process_group_lifecycle.py` observe la sortie du chef avec `waitid(WNOWAIT)`, afin de garder son PID réservé pendant la lecture de son groupe dans `/proc`. Si des descendants actifs restent dans le même groupe, le lanceur leur envoie SIGTERM puis SIGKILL avec délais bornés; il réapure ensuite le chef et classe l’étape en `125` (inconclusive). Une impossibilité de confirmer le nettoyage échoue aussi fermée. Les deux chemins concernés sont `run_stage_command` et `run_bounded_cli`.

Cela ferme le faux succès observé et le processus du test est mort avant le retour des deux lanceurs. Le travail porte sur les lanceurs locaux de preuve; il n’ajoute ni AgentOS ni un runtime de modèle.

## Vérification

Environnement : Python 3.12.3, Linux `7.0.0-1019-nvidia`.

- `python3 -m unittest discover -s tests -p 'test_mission_012_live_composed_e2e.py'` — 7/7 réussis.
- `python3 -m unittest discover -s tests -p 'test_mission_012_live_mcp_agent_e2e.py'` — 10/10 réussis.
- `python3 -m py_compile scripts/process_group_lifecycle.py scripts/run_mission_012_live_composed_e2e.py scripts/run_mission_012_live_mcp_agent_e2e.py tests/test_mission_012_live_composed_e2e.py tests/test_mission_012_live_mcp_agent_e2e.py` — réussi.
- `git diff --check` et contrôle Python de whitespace/NUL sur les cinq fichiers de code — réussis.

Les tests verts sont consignés dans `tdd-green-process-runners.txt`; les sources sont liées par `source-hashes.txt` et le manifeste local. Aucun test Cargo n’est relancé puisque ce lot ne change pas de Rust, de dépendance ni de contrat Cargo.

Le premier `sha256sum -c` local a été lancé depuis la racine du worktree, où les chemins du manifeste local ne se résolvent pas; il a échoué pour ce motif. La reprise depuis le dossier de preuve a passé avant la dernière édition du présent rapport. L’index append-only garde ensuite l’ancienne empreinte en plus de la nouvelle; le `sha256sum -c` brut signale donc l’ancienne ligne de `local-run.md`, tandis que `verify_append_only_indexes.py` valide la dernière empreinte pour chaque chemin et les préfixes. Les commandes et résultats successifs sont consignés dans `manifest-check.txt`.

## Limites

Preuve synthétique `n=1` par lanceur, sur Linux avec `/proc`; pas de LLM, AgentOS, Forgejo, PR, gate de fusion ou comportement post-merge. Le contrôle couvre les descendants qui restent dans le groupe de processus de l’étape. Il ne détecte pas un descendant qui crée délibérément une nouvelle session, un processus extérieur au groupe, un acteur privilégié ni une évasion de sandbox/cgroup. Il s’agit d’un contrôle de cycle de vie du harnais et non d’une frontière de sécurité générale. Le test-adapter local Bubblewrap conserve son namespace PID, mais cette preuve n’exerce pas une session détachée.

Pas de dépense, de réseau externe, de production, de publication, de suppression ou de changement de licence. La licence Cargo reste non choisie; le parcours LLM/AgentOS répété et la comparaison contrôlée restent non démontrés. Projet incomplet.
