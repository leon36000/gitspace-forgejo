# CONTINUATION-440 — préflight et correction de collecte

Date : 28 octobre 2026. Worktree PC6, branche `local/WEBHOOK-005-canonical-links`, HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`.

Le préflight préparait le contrôle natif ciblé de CONTINUATION-438. Le premier collecteur a échoué avec `KeyError` après les sondes, avant tout lancement de Forgejo. Le scan large de `/tmp` a aussi rencontré des répertoires protégés; ses chaînes de chemin sont omises du reçu. Ce scan cherchait le nom exact `forgejo` et ne pouvait donc pas trouver `/tmp/forgejo-release-slash-001`.

La vérification ciblée a établi que ce binaire est un fichier régulier non lié, exécutable, avec le SHA-256 `e5ec437cd6357e8d1b4580cb47ab1daa58a32b33971a63db63446a535d33adb7` déjà présent dans le reçu natif CONTINUATION-433. `--version` retourne **forgejo version 17.0.0+gitea-1.22.0 (release name 17.0.0) built with GNU Make 4.4.1, go1.26.9 : bindata, timetzdata, sqlite, sqlite_unlock_notify**; ce hash/version ne révèlent pas le commit Go de build. Le port 3130 était libre lors du préflight. Les sorties sûres et l’erreur du collecteur sont consignées sans détail de répertoires protégés.

Aucun test ni serveur n’a été lancé dans ce lot de préflight. Le contrôle natif, exécuté ensuite dans un nouvel état privé et un nouveau dossier de preuves, est le reçu [CONTINUATION-441](../continuation-441-forgejo-native-branch-delete-001/local-run.md). Cette note ne prétend pas à l’absence d’un autre binaire ailleurs ni à un blocage du runner.
