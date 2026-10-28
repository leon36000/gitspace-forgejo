# Observation de couverture avant raccordement

Commande : `python3 -m unittest discover -s tests -p 'test_mission_012_sandboxed_test_adapter.py' -k test_probe_process_remains_isolated_until_reprobe_and_release -v`.

Le premier essai a échoué (exit 1) sur l’assertion exacte de `private_runtime_files_not_visible` : l’attendu incluait `host_admission`, mais la liste réelle n’incluait pas ce label puisque le fichier canari n’était pas encore passé au probe. Ce n’était pas un échec du sandbox ni la preuve d’une fuite; c’était le test de couverture inachevé. Après création du canari privé et raccordement au probe, le scénario passe. La sortie rouge complète n’a pas été sauvegardée; cette note résume l’observation directe de la sortie d’outil.
