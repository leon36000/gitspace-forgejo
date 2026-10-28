# Historique des exécutions locales du harnais

Ces dossiers gardent les itérations qui ont servi à construire et durcir le test; le verdict à citer est le run 007, qui contient les sources, logs Cargo et observations de credential/namespaces les plus complètes.

- `stdio-mcp-001` : préflight refusé avant démarrage de Forgejo parce que la commande avait redirigé ses sorties dans le répertoire d’evidence. Aucun état `/tmp` créé; détail conservé dans `attempt-note.md`.
- `stdio-mcp-002` : première composition réussie processus `--stdio-mcp` + routes live + lecture de gate. Les fichiers Cargo du dossier sont des résumés initiaux; ne les interpréter pas comme les sorties Cargo complètes.
- `stdio-mcp-003` : sorties réelles `cargo test --offline --locked` ajoutées aux artefacts.
- `stdio-mcp-004` : snapshot des helpers et des sources fixture ajouté pour améliorer la reproductibilité.
- `stdio-mcp-005` : assertion explicite que le wrapper reste vivant après la lecture de gate; token de gate encore limité à la lecture.
- `stdio-mcp-006` : premier essai avec un token Forgejo local à scope `write:repository` dans un `memfd CLOEXEC`, et son chemin host `/proc` sondé depuis le PID namespace.
- `stdio-mcp-007` : run final; scopes read/write complets utilisés par le gate, namespaces/variables d’environnement vérifiés, aucun canari fichier factice créé par le runner, fichiers source, logs Cargo et rapport intégral conservés.

Les runs 002–007 ont chacun démarré et arrêté leur propre instance Forgejo jetable sur le port loopback 3130 avec des répertoires d’état distincts sous `/tmp`; aucun merge n’a été réalisé par ces runs.
