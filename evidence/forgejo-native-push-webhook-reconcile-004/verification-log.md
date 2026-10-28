# Preuve de vérification — FORGEJO-NATIVE-PUSH-PARTIAL-SELECTION-005

Ces sorties accompagnent le rapport d’exécution principal `../forgejo-native-push-webhook-reconcile-005/local-run.md`.

- La première exécution RED de `PushPartialSelectionRefusalTests` a échoué sur l’assertion attendue « partial push batch refusal validator is missing » (4 tests, avant ajout de l’implémentation). Après l’ajout, cette classe a passé 4/4.
- La suite `test_forgejo_emitted_webhook_e2e.py` a passé 38/38.
- `py_compile` a passé pour le runner et le fichier de tests.
- `cargo test --locked --offline` a passé : 510 réussis, 0 échec, 61 ignorés, 35 groupes.
- Les 7 tests du garde de licence Cargo ont passé; l’état reste `license=null`, `license_file=null`, `publish=[]`, redistribution `not_assessed`.
- `git diff --check` a passé.

Les commandes et sorties détaillées sont dans les fichiers voisins. `e2e-console.json` conserve seulement l’invocation refusée par le garde anti-écrasement de preuves; elle n’a démarré aucun service. Le parcours réussi est séparé dans le dossier 005.
