# CONTINUATION-222 — annulation de stage et reprise dans un scope neuf

Date : 2026-10-27. Exécution locale de test sous Linux/aarch64 avec le systemd user manager existant. Aucun accès externe, instance Forgejo, modèle ou coût fournisseur.

## Scénario et résultat

`test_attested_parent_scope_cancellation_cleans_then_fresh_scope_restarts_stage` lance réellement un harness Python dans un scope systemd transient créé et attesté par l’hôte. Depuis ce harness, `run_stage_command_in_attested_parent_scope` lance un stage qui reste en attente. Le test envoie `SIGINT` au processus du harness. Le handler d’interruption du runner termine le groupe du stage, exécute le cleanup et compare l’ensemble actif du cgroup à son snapshot initial. Le harness observe `KeyboardInterrupt` seulement après le nettoyage et termine; le contrôleur obtient l’état systemd `clean`, et le PID du stage n’est plus actif.

Le contrôleur ouvre ensuite un scope systemd distinct. Un stage frais y vérifie à nouveau l’attestation du parent, imprime `fresh-scope-recovery` et termine code 0. Le test vérifie que les noms de scope d’annulation et de reprise diffèrent. Cela démontre le retour d’un runner de processus à un scope neuf après interruption; ce n’est pas une reprise d’une mission ni d’un checkpoint métier persistant.

## Échec de test observé et correction

Le premier essai du nouveau test a progressé jusqu’à la phase de reprise, puis a levé `ValueError: I/O operation on closed file` : `launch_scoped_process` ferme `stdin` après son protocole d’attestation, alors que `Popen.communicate()` essayait de le vider une seconde fois. La collecte de sortie du test lit maintenant directement `stdout`, attend le processus et ferme le pipe. Le test ciblé passe ensuite sans avertissement.

Au premier lancement de la suite composeur complète, le test existant `test_agent_cli_uses_nested_scope_and_refuses_a_detached_survivor` a échoué parce que le fichier PID de l’enfant détaché n’avait pas encore été écrit. Son parent CLI finissait sans attendre cette écriture; le scope le terminait donc pendant cette course. La fixture attend maintenant ce reçu borné avant la sortie. L’échec initial reste dans `composed-tests.log`; le rerun complet `composed-tests-green.log` passe 18/18. Cette correction stabilise l’observation du test et ne relâche aucun contrôle de scope.

## Vérifications

- Test d’annulation/reprise ciblé : 1/1.
- Suite `test_mission_012_live_composed_e2e.py` : 18/18 après la correction de la fixture.
- Suite `test_mission_012_live_mcp_agent_e2e.py` : 13/13.
- `py_compile` sur les quatre scripts concernés : code 0.
- `git diff --check` : code 0.
- `verify_cargo_license_state.py` : `license=null`, `license_file=null`, `publish=[]`, `no_license_selected`, droits `not_assessed`, `NOASSERTION`.

Les sorties sont archivées dans les fichiers `.log` de ce dossier. Les hashes des fichiers modifiés et versions figurent dans `source-sha256.txt` et `tool-versions.txt`.

## Portée restante

La reprise redémarre une commande synthétique sous un nouvel identifiant systemd. Elle ne reprend pas l’état persistant d’une demande, d’une mission, d’une tentative d’agent, d’une PR ou d’une preuve; elle ne prouve pas l’invalidation/revalidation métier après interruption. Aucun Forgejo, MCP réel, LLM ou AgentOS n’est exécuté. Le test couvre SIGINT sous ce système et ce cgroup, pas tous les signaux, noyaux, gestionnaires ou acteurs privilégiés. Le parcours CONTINUATION-221 n’a pas été rejoué et n’est pas élargi par cette preuve.

Prochaine lacune : examiner le vrai format et les transitions existantes des checkpoints de mission afin de tester une reprise versionnée avec preuves périmées refusées, sans inventer un second moteur de mémoire. Le benchmark apparié, le runtime AgentOS/LLM autorisé, les droits/licences et l’exclusion universelle de writers restent ouverts; le projet est incomplet.
