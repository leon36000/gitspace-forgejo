# WEBHOOK-AUTH-FAILURE-BATCH-STOP-ROTATION-001 — arrêter un batch après le refus du credential

Date : 25 octobre 2026. HEAD observé : `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`; le worktree est fortement modifié. Les sources précises et les artefacts sont hashés dans `source-sha256.txt` et `SHA256SUMS`.

## Reproduction rouge

Le nouveau scénario `actual_reconcile_cli_process_stops_batch_reads_after_401_and_resumes_with_rotated_credential` a d’abord échoué contre le comportement précédent. Deux webhooks HMAC signés visant des issues distinctes du dépôt configuré ont été admis dans le journal. La fixture a renvoyé HTTP 401 pour la première issue. Le processus a ensuite émis un second GET pour la sœur avec le même credential. L’assertion rouge a observé `left: 2`, `right: 1` pour le nombre de requêtes. L’extrait conservé est dans `tdd-red-observation.txt`.

## Changement et résultat vert

`LocalLoopbackForgejoApi` mémorise désormais le premier `Unauthorized` pour le reste de cette instance de client. Après un 401, une nouvelle ressource dans le dépôt configuré reçoit le même résultat terminal `upstream_unauthorized` depuis le client, sans renvoyer le token. La couche cache batch garde le résultat par ressource; aucun résultat n’est canonique et le verdict batch reste `inconclusive`. Un refus `Forbidden` d’un dépôt hors scope est contrôlé avant le latch. Une invocation ultérieure construit une nouvelle instance, lit un token synthétique renouvelé depuis un pipe privé, relit les deux IDs et reçoit `200/open` pour les deux. Le test vérifie l’ordre, les compteurs de requêtes, l’absence du token et du corps d’erreur dans stdout/stderr et l’égalité octet-à-octet du journal avant/après chaque processus.

Vérifications finales : le scénario nouveau passe `1/1` (45 tests filtrés); le scénario existant 401 sur deux livraisons identiques passe `1/1`; le scénario 403 sur une sœur réussie passe `1/1`. La suite `cargo test --offline --locked --all-targets -- --test-threads=1` passe 480 tests, 0 échec, 52 ignorés, en 34 groupes; sortie complète dans `all-targets.log`. Rustfmt, `git diff --check` et le garde Cargo passent.

## Limites et état de licence

Le latch est un comportement conservateur : un 401 pour une ressource rend les lectures suivantes du même client/dépôt inconclusives, même si une route sœur aurait éventuellement répondu différemment. Il est en mémoire par invocation; une nouvelle invocation permet la reprise avec un credential corrigé. Ce test process-level utilise le receiver et le CLI réels, mais un serveur HTTP loopback synthétique — pas une instance Forgejo. Il ne teste pas plusieurs processus simultanés, l’expiration réelle d’un token, ni le cas combiné d’un 401 suivi d’un événement hors scope (la validation scope précède le latch dans le code). Aucun modèle/AgentOS/RAG/mémoire, credential réel, service externe, coût modèle, publication ou production n’est utilisé.

`cargo metadata --offline --locked --no-deps --format-version 1` et `scripts/verify_cargo_license_state.py` confirment le paquet `agent-forge-core` version `0.1.0` avec `license=null`, `license_file=null`, `publish=[]`, `no_license_selected` et `redistribution_rights=not_assessed`. Aucune licence produit n’est déduite ni choisie.
