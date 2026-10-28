# Essais et diagnostics de FORGEJO-NATIVE-BRANCH-CREATE-001

Ce dossier conserve le premier parcours natif réussi et les essais de vérification qui ont conduit au rapport final dans `../forgejo-native-branch-create-002/`.

- Le premier `cargo test --all-targets` a échoué dans `mcp_supervisor_cancellation_terminates_the_in_flight_verifier_scope`: l'assertion attendait un cgroup vide immédiatement après la sortie du superviseur et a observé brièvement un PID. Le test isolé a passé. Le contrat attend maintenant jusqu'à deux secondes, avant son garde de nettoyage; un run complet final a passé.
- Deux suites suivantes ont atteint des attentes devenues obsolètes lorsque `create` est passé de non pris en charge à pris en charge. Le test HTTP a été changé de `create` à `repository`, et le test reconciler de `create` à `star`, pour garder de vrais événements inconnus.
- Le run complet final, après ces corrections, est archivé dans le rapport 002: 525 réussis, 0 échec, 64 ignorés, 34 groupes. Le périmètre `--all-targets` exclut doctests et tests marqués ignored.

Les journaux non réussis ne sont pas supprimés ni présentés comme résultats positifs. Le changement d'attente du cgroup est limité au test de processus; aucune logique du superviseur ou du runtime n'a été modifiée dans ce lot.
