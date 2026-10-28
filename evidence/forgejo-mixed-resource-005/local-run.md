# FORGEJO-MIXED-RESOURCE-005 — lot local mixte avec reprise après création

## Résultat

Dans Forgejo 17.0.0 local et jetable, le reconciler reçoit deux identifiants distincts pour deux `ResourceRef` d'issue dans le même dépôt. Le premier identifiant pointe vers une issue #2 encore absente et le second vers l'issue #1 existante. L'observateur HTTP indépendant voit, dans cet ordre, `GET issues/2 → 404` puis `GET issues/1 → 200`, tous deux avec le credential read-only attendu. Le CLI sort 1 et retourne `inconclusive`: l'erreur `resource_not_found` est non réessayable et actionnable, tandis que l'issue #1 reste lue canoniquement. Deux lectures distinctes sont comptées.

Après ce refus, le propriétaire crée réellement l'issue #2. Forgejo émet ensuite un événement `issues/opened` natif; le proxy le lie au task Forgejo et le receiver l'ajoute au journal. Le même lot de deux identifiants (même ordre) est relancé. L'observateur voit alors deux `GET → 200`, et le CLI sort 0 avec `current_state_read` pour les deux ressources. Le journal est inchangé pendant chacune des deux réconciliations.

La première entrée du lot est un événement HMAC synthétique envoyé directement au receiver local pour représenter l'issue absente avant sa création. Ce n'est pas un task webhook émis par Forgejo. La seconde entrée est une livraison native rejouée par Forgejo. Après création, le webhook Forgejo natif de l'issue #2 est également reçu et vérifié, mais le test relance volontairement les deux UUID initiaux. Cette limite est conservée dans [mixed-resource-reconciliation.json](mixed-resource-reconciliation.json); le test ne présente pas les deux entrées initiales comme des livraisons natives.

## Environnement et vérifications

- Source produit : HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`, worktree déjà fortement modifié.
- Forgejo : binaire `17.0.0+gitea-1.22.0`, SHA-256 `e5ec437cd6357e8d1b4580cb47ab1daa58a32b33971a63db63446a535d33adb7`. Source officielle examinée au commit `0d791421f85ca13a63b665053d81ea765fbbc571`; le commit exact de build du binaire n'est pas établi.
- Le runner a recompilé le receiver et reconciler Rust offline/locked sous Bubblewrap 0.9.0, avec namespaces réseau/PID et source/sysroot en lecture seule. Les empreintes sont dans [build-provenance.json](build-provenance.json).
- Tests ciblés du validateur : 25/25; suite Python : 189/189; tests d'état de licence Cargo : 7/7. `cargo fmt --all -- --check`, `py_compile` des deux fichiers Python modifiés, `git diff --check` et le readback Cargo passent. La suite Rust complète n'a pas été exécutée.
- Le readback Cargo garde `license=null`, `license_file=null`, `publish=[]`, `no_license_selected` et `redistribution_rights=not_assessed`; aucune licence n'est choisie.
- Forgejo, receiver, proxies et listeners sont arrêtés après le run; `process_cleanup.cleanup_failure=null`, aucun listener ne reste sur 3130/3131. Le répertoire d'état du serveur est privé sous `/tmp` et n'est pas livré avec les preuves.
- Le manifeste local 005 vérifie ses 19 artefacts; celui du lot 004 réparé vérifie 27/27. Après ajout append-only, les 53 chemins source/document/preuve pertinents ont zéro divergence à leur dernière occurrence dans le manifeste global. Au contrôle avant le hash final de ce rapport, ce journal avait 5 826 lignes : 5 820 entrées SHA-256 et six lignes historiques malformées, conservées sans réécriture; aucun chemin pertinent n'était absent.

Commandes et sorties synthétiques : [command.txt](command.txt), [toolchain.txt](toolchain.txt), [verification-summary.txt](verification-summary.txt). Le résultat structuré est [result.json](result.json); les observations de requêtes sont dans [api-observation-proxy.json](api-observation-proxy.json). Le manifeste local de ce dossier vérifie tous ses artefacts.

## Portée et suite

Cette expérience prouve le traitement ordonné de deux issues distinctes par le vrai CLI contre une instance Forgejo locale, l'isolation d'un 404 sur l'issue #2, puis la reprise des mêmes identifiants après création de la ressource. Elle ne prouve pas une déduplication durable des erreurs entre processus, une charge concurrente, une récupération après crash, un agent LLM/AgentOS, un gain comparatif ou une intégration en production. Le premier événement étant synthétique, une livraison native d'une ressource ensuite absente reste à étudier. Le projet reste incomplet.
