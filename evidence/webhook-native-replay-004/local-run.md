# WEBHOOK-NATIVE-REPLAY-004 — replay natif Forgejo v17 et binding du payload

Date : 24 octobre 2026. Instance locale isolée; dépôt privé jetable `forge-admin/agent-forge-core`, issue #1. Ce run exerce le replay manuel de Forgejo par sa session web et relie le corps JSON reçu au payload stocké par Forgejo.

## Commande exacte

```sh
PYTHONDONTWRITEBYTECODE=1 python3 scripts/run_forgejo_live_webhook_native_replay_e2e.py \
  --forgejo-binary /tmp/forgejo-17.0.0 \
  --repo-root /home/pc6/Documents/Codex/2026-10-21-mission-construire-une-forge-con-ue/.runtime/webhook-005-workspace \
  --state-dir /tmp/forgejo-native-replay-004-state \
  --evidence-dir /home/pc6/Documents/Codex/2026-10-21-mission-construire-une-forge-con-ue/.runtime/webhook-005-workspace/evidence/webhook-native-replay-004
```

Le runner compile le receiver Rust depuis l’arbre de travail avec Bubblewrap, source/registry staged en lecture seule, `--offline --locked`, namespaces réseau/PID et environnement vidé. Forgejo écoute sur `127.0.0.1:3130`; le receiver écoute sur loopback. Comptes, secret, SQLite et logs privés restent dans `/tmp/forgejo-native-replay-004-state` et sont conservés.

## Résultat mesuré

- Forgejo `17.0.0+gitea-1.22.0`, binaire SHA-256 `e5ec437cd6357e8d1b4580cb47ab1daa58a32b33971a63db63446a535d33adb7`. Le code source officiel v17.0.0 examiné est commit `0d791421f85ca13a63b665053d81ea765fbbc571`; aucun lien de provenance du build du binaire à ce commit n’est prouvé.
- La livraison initiale `issues/opened` échoue en HTTP 401 avec un secret volontairement erroné et n’écrit rien dans le journal du receiver. Une observation de 1 s voit toujours une seule tâche, mais le harnais ne compte pas les requêtes HTTP; elle ne prouve donc pas l’absence d’une répétition du même task.
- Sans session, replay renvoie 303 vers `/user/login`; authentifié mais cross-origin, il renvoie 403; avec UUID inexistant, 404. Chaque refus laisse une seule tâche.
- Deux POST same-origin authentifiés sur le même UUID initial échoué renvoient chacun 303. Forgejo crée deux tasks et UUID nouveaux et distincts. Le SHA brut du payload stocké reste identique; en plus, le runner parse `payload_content` de chaque task et compare son SHA-256 JSON canonique au payload JSON du reçu receiver. Les deux bindings concordent (`d803a3ef61da2d63945593785fb430595159799acccc77cd5a49293588f67a45`). Les deux livraisons HMAC-valides sont reçues en HTTP 202, séquences 1 et 2, pour la même occurrence métier `issue #1 / opened`. Le replay POST répété n’est pas idempotent.
- Le readback API confirme le hook inactif; Forgejo et les receivers suivis ont quitté; aucun listener subsiste sur les ports contrôlés.

Les reçus détaillés sont dans `result.json`, `native-replay.json`, `hook-tasks.json`, `webhook-events.json`, `hook-readback.json` et `provenance.json`; la [revue indépendante read-only](independent-review.md) ne relève aucun défaut matériel restant. `SHA256SUMS` couvre tous les fichiers du dossier.

## Vérifications

- `PYTHONDONTWRITEBYTECODE=1 python3 -m unittest -v tests.test_forgejo_webhook_native_replay_e2e` : 6 tests passent, dont payload JSON sémantiquement modifié, UUID dupliqué et séquence incorrecte refusés.
- `PYTHONDONTWRITEBYTECODE=1 python3 -m py_compile scripts/run_forgejo_live_webhook_native_replay_e2e.py tests/test_forgejo_webhook_native_replay_e2e.py` passe.
- Les SHA-256 du runner et des tests correspondent à `provenance.json`. Provenance Rust : Cargo/rustc 1.95.0, Bubblewrap 0.9.0, compilation hors ligne et verrouillée.
- La tentative de harnais 001 a échoué sur un mauvais module de vérification du listener avant le replay web; elle reste archivée et n’est pas comptée comme succès. La run 002 a prouvé un replay unique; 003 a ajouté les refus HTTP et le double POST; 004 lie désormais le contenu JSON complet sémantique des tasks au contenu reçu.

## Limites

Ce test ne réalise pas la chaîne demande → exigences → mission → agent → PR → vérificateur indépendant → fusion. Aucun LLM, AgentOS, Forge Code, RAG ni mémoire n’est utilisé; aucun gain général, coût de tokens ou durée comparée n’est mesuré. `n=1`, compte admin local; aucun compte non-admin ni test Go Forgejo. Le readback `hook_task` dépend du schéma SQLite interne à Forgejo v17. Forgejo et receiver partagent l’UID hôte; Bubblewrap limite le build, pas les privilèges des services. L’identité de commit source du binaire local reste inconnue. Les empreintes attestent une identité de contenu/provenance observée, pas la correction du programme.
