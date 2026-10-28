# CONTINUATION-131 — refus d’un batch résoluble sur une inbox tronquée

Date : 26 octobre 2026. Worktree `/home/pc6/Documents/Codex/2026-10-21-mission-construire-une-forge-con-ue/.runtime/webhook-005-workspace`, HEAD de base `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`, branche `local/WEBHOOK-005-canonical-links`. Le dépôt était fortement modifié; aucun fichier n’a été supprimé, réinitialisé, commité ou publié.

## Lacune et correction de couverture

CONTINUATION-130 couvrait une inbox tronquée avec une livraison connue et un ID demandé absent. Même si le CLI historique réparait la queue en appelant `replay()`, la sélection échouait ensuite sur l’ID absent avant le credential et avant l’API. Ce cas ne démontrait donc pas le refus d’une batch dont toutes les sélections se résolvent.

Le nouveau test processus `real_cli_refuses_a_torn_batch_when_every_selected_delivery_exists` crée deux événements d’issue dans le préfixe valide, vérifie qu’ils sont tous deux lisibles, puis ajoute un record incomplet. Il lance le vrai `forgejo-reconcile-local` avec ces deux IDs et un FD de credential synthétique. Bubblewrap isole le réseau; le test autorise en écriture uniquement le scratch temporaire nécessaire au verrou de l’inbox. Une sonde exécutée dans le namespace ne rejoint pas `127.0.0.1:3130` (`ConnectionRefusedError`, errno 111).

Le processus retourne code 1 et le schéma batch 6 `inconclusive/inbox_journal_invalid`, avec deux IDs demandés, zéro lecture amont, `host_credential_read=false`, `upstream_calls_attempted=false` et `journal_repair_attempted=false`. Le FD conserve le credential synthétique en entier et les octets de l’inbox sont identiques avant/après. Une régression vers l’ancien `replay()` aurait réparé la queue puis pu poursuivre le traitement des deux événements; le namespace réseau empêche alors le binaire de toucher à un service Forgejo local. Le test observe bien l’effet du CLI isolé, pas cette ancienne version du code.

## Vérifications

`cargo test --offline --locked --test webhook_quarantine_cli` passe 5/5, dont les quatre tests précédents et ce nouveau scénario. `cargo fmt --all -- --check` et `git diff --check` passent. Aucun code runtime ne change; `cargo test --all-targets` n’est donc pas répété. Logs, versions, commit de base et hashes source sont conservés dans ce dossier.

Le premier appel d’append des manifestes a refusé d’écrire parce que le préfixe « project » avait été capturé depuis le chemin worktree relatif, au lieu du chemin résolu de l’index parent. L’échec est survenu avant tout append. Après correction du préfixe à partir de `/home/pc6/Documents/Codex/2026-10-21-mission-construire-une-forge-con-ue/evidence/SHA256SUMS`, le script vérifie 17 fichiers locaux et les trois préfixes; 20 entrées worktree et 21 entrées projet ont été ajoutées, avec les anciens préfixes intacts.

La première tentative de test a rendu le `/tmp` de Bubblewrap entièrement lecture seule et échoué sur la création du verrou (`Io("Read-only file system")`). Le harnais a été ajusté pour bind-monter en écriture uniquement le répertoire scratch du test. Cette erreur était un défaut du montage de test, non un défaut produit; le run final ci-dessus est postérieur à cette correction.

## Portée et limites

Inbox, IDs et contenu sont synthétiques; aucun Forgejo, LLM, AgentOS, secret réel, appel externe ou dépense n’est impliqué. Le test prouve le refus d’une queue finale incomplète avant l’API et la lecture du credential, pour un batch de deux événements valides, sous Linux/Bubblewrap. Il ne prouve pas l’absence de writers extérieurs qui ignorent le verrou, la réparation/récupération contrôlée d’une queue, la concurrence sémantique, un parcours LLM ni un gain comparatif.

Projet toujours incomplet. Prochaine action : reprendre une autre lacune indépendante de la matrice. Les critères de benchmark exigent encore un corpus autorisé, un agent/runtime accessible et un plafond de coût contraignant; le profil Forgejo multi-writer reste refusé sans CAS serveur universellement appliqué.
