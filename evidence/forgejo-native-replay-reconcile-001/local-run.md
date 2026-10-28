# FORGEJO-NATIVE-REPLAY-RECONCILE-001

## Résultat

Un parcours loopback sur Forgejo 17.0.0 a confirmé le traitement de deux relectures manuelles d'un même webhook d'issue. Les deux livraisons ont des UUID distincts, mais conservent le même payload sémantique. Le receiver les journalise séparément. Le CLI de réconciliation relit l'issue privée par un credential de lecture non-admin, associe les deux lignes aux UUID demandés, retourne l'état canonique `open` pour chacune et effectue une seule lecture API upstream. Les octets du journal durable sont identiques avant et après la réconciliation.

Le test du validateur batch refuse les résultats réordonnés ou substitués, plus d'une lecture upstream et un état canonique différent du readback Forgejo. Les résultats sont liés à l'identité owner/repository/issue, au numéro d'issue, à l'URL, à `updated_at` et à l'état lu.

## Commandes et versions

Commande reproductible : [command.txt](command.txt). Le runner a construit les deux binaires Rust hors ligne avec Cargo `1.95.0` et Rust `1.95.0`, puis démarré Forgejo depuis `/tmp/forgejo-17.0.0`. Python `3.12.3`, Bubblewrap `0.9.0`; le binaire rapporte Forgejo `17.0.0+gitea-1.22.0`, construit avec Go `1.26.9`. Le binaire Forgejo a SHA-256 `e5ec437cd6357e8d1b4580cb47ab1daa58a32b33971a63db63446a535d33adb7`; la provenance ne rattache pas son build au commit source audité `0d791421f85ca13a63b665053d81ea765fbbc571`.

Sources du projet examinées au HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0` (worktree sale) : `scripts/run_forgejo_live_webhook_native_replay_e2e.py`, `scripts/run_forgejo_native_push_webhook_reconcile.py` et `tests/test_forgejo_webhook_native_replay_e2e.py`. SHA des sources d'exécution consignés dans [provenance.json](provenance.json). Le test ciblé passe 13/13; la suite Python passe 177/177. Les deux binaires ont été construits par le runner. `cargo fmt --all -- --check`, `git diff --check`, `py_compile`, les 7 tests du garde de licence et le readback `cargo metadata --locked --offline` passent. Aucun choix de licence n'est fait (`license=null`, `license_file=null`, `publish=[]`, droits de redistribution `not_assessed`). La suite Rust complète n'a pas été relancée dans ce lot.

Le test d'acceptation vert est archivé dans `focused-tests.log`; la suite Python complète dans `python-tests.log`. Le test rouge TDD précédant l'ajout du validateur a été observé lors de l'implémentation, avec quatre échecs attendus faute d'interface, mais sa sortie brute n'a pas été conservée et n'est donc pas présentée ici comme un artefact reproductible.

## Observations et limites

- Une requête initiale avec HMAC invalide retourne 401 et ne crée aucune ligne du journal. Le replay natif par l'interface Web est testé contre l'instance locale; le replay sans session retourne 303, le cross-origin 403 et l'UUID inconnu 404.
- Deux POST de replay du même UUID d'origine créent deux tâches Forgejo et deux nouvelles livraisons; la sémantique des payloads est identique. Les deux événements reçus répondent 202.
- La réconciliation utilise un credential read-only privé dans un FD hérité du processus. Les artefacts n'enregistrent pas sa valeur. La requête API distincte de readback et les deux résultats du batch concordent; `upstream_read_attempts=1`.
- Le replay d'un hook inactif crée une tâche Forgejo marquée comme délivrée/échouée, sans tentative proxy ni nouvelle ligne du journal.
- Le runner désactive le hook, arrête les processus suivis et vérifie l'absence de listeners; `cleanup_failure=null`. Le répertoire temporaire d'état a été conservé sous `/tmp/forgejo-native-replay-reconcile-001-state`.

Un dépôt, une issue et deux replays dans une instance jetable, sous l'UID local. Le validateur Python contrôle la forme du résultat du CLI et un readback indépendant; ce n'est ni une autorité de fusion ni une preuve d'idempotence métier pour toutes les catégories d'événements. Le binaire Forgejo n'est pas lié cryptographiquement à la source auditée. Le run ne teste ni LLM, ni runtime AgentOS, ni code hostile, ni concurrence sous charge, ni comparaison contrôlée, ni effet post-fusion. Il ne démontre aucun gain général. Aucun appel de service externe, coût de modèle, déploiement ou modification de production.

Les preuves détaillées sont dans `result.json`, `reconciliation.json`, `webhook-events.json`, `hook-tasks.json`, `native-replay.json`, `http-proxy.json`, `cleanup-readback.json`, `provenance.json`, `build-provenance.json` et les journaux des vérifications. Leur manifeste se trouve dans [SHA256SUMS](SHA256SUMS).

## Suite utile

Préparer localement le protocole d'une comparaison appariée sans encore appeler de modèle : tâches, critères d'acceptation protégés, limite d'interventions et schéma de collecte coût/temps/tokens fixés avant exécution. Pour une comparaison avec LLM, attendre un budget numérique fiable et explicitement autorisé. Continuer en parallèle les cas de refus et de reprise non couverts, sans répéter le replay natif déjà prouvé. Le projet reste incomplet.
