# MISSION-012-COMPOSED-012 — admission avec replay natif (24 octobre 2026)

## Résultat

Le composeur expose maintenant l’option explicite `--native-replay-admission`. Elle transmet le mode au runner d’admission Forgejo et valide, avant l’étape agent puis PR, deux livraisons `issues` acceptées en HTTP 202 avec UUID distincts, le même hash brut et sémantique, et le hash du payload reçu relié aux tasks. Le second UUID doit réutiliser la mission, l’allocation et la génération déjà admises; les hashes avant/après du journal et du fichier d’admission doivent rester égaux. Sans l’option, la présence d’un replay supplémentaire est refusée. L’option est propagée aux validations d’étape et au résumé final; le comportement par défaut garde une seule livraison.

Le test rouge initial montrait que les deux nouveaux paramètres n’étaient pas encore pris en charge. Après implémentation, les 50 tests ciblés passent; `py_compile`, `git diff --check` et l’aide CLI confirment le code et l’option. Sortie : [tests ciblés](../mission-012-live-agent-webhook-native-replay-038-focused-tests-final.log).

## Tentative E2E et blocage

Commande tentée sur Forgejo `17.0.0+gitea-1.22.0`, binaire local SHA-256 `e5ec437cd6357e8d1b4580cb47ab1daa58a32b33971a63db63446a535d33adb7`, avec `--agent-adapter local-test-adapter`, `--native-replay-admission`, cap configuré `0.20` et état privé neuf sous `/tmp/mission012-native-replay-038`. Le premier appel avec chemin d’évidence relatif a été refusé par le contrôle d’argument avant création d’état; le second utilisait le chemin absolu attendu.

Ce second run s’est arrêté à `build_trusted_local_adapters` avec `OSError: [Errno 28] No space left on device`, avant le démarrage de Forgejo et avant toute admission ou étape agent/PR. L’évidence partielle de l’admission dit `verdict=failed`; le contrôle de ports rapporte aucun listener résiduel. Voir [failure.json](../mission-012-live-agent-webhook-native-replay-038-admission/failure.json), [lecture de nettoyage](../mission-012-live-agent-webhook-native-replay-038-admission/port-cleanup-readback.json) et [manifeste local](../mission-012-live-agent-webhook-native-replay-038-admission/SHA256SUMS).

Au contrôle du système de fichiers racine (`/tmp` et le worktree sont sur le même volume), `df -h` indiquait 916 Go au total, 869 Go utilisés et 1,7 Mo libres; le volume était à 100 %, avec 27 % des inodes utilisés. Après la tentative, il restait environ 1,4 Mo. Le répertoire privé de cette tentative n’a pas été nettoyé. Un parcours E2E composé incluant le replay natif n’est donc pas démontré par ce lot.

Les preuves préexistantes restent séparées : [l’admission live avec replay natif 005](../mission-native-replay-admission-005/local-run.md) et [le parcours composé 037](../mission-012-live-agent-webhook-local-adapter-037/local-run.md). Elles ne remplacent pas le run composé manquant.

## Portée et suite

Le run prévu utilise uniquement le `local-test-adapter`; aucun LLM, AgentOS, Forge Code, RAG ou moteur de mémoire n’a été appelé et aucun coût modèle n’a été engagé. La CLI Claude Code installée est `2.1.310`, mais le budget pour un nouvel appel reste en attente de clarification; l’ancien dépassement estimatif de 0,20 USD est documenté dans le checkpoint. Un sous-agent de surveillance a repris deux relevés espacés d’environ 90 secondes : 26 processus `claude` conservent PID/PPID/état, leurs âges augmentent de 1 min 34–35 s et le CPU augmente de 1 s pour 9 processus, sans changement pour 17. Cela montre une faible activité CPU, pas une progression utile. Aucun processus n’a été contacté ou arrêté.

Prochaine action : après libération autorisée d’espace disque, utiliser un nouveau nom d’état et d’évidence et rejouer la commande avec le chemin d’évidence absolu. La licence Cargo reste `null`, sans licence choisie; les droits de redistribution de GitSpace demeurent non évalués. Le worktree source est dirty au HEAD examiné `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`.
