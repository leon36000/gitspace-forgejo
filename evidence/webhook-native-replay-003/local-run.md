# WEBHOOK-NATIVE-REPLAY-003 — replay natif Forgejo v17

Date : 24 octobre 2026. Exécution locale isolée, dépôt privé jetable `forge-admin/agent-forge-core`, issue #1. Cette preuve porte sur la route web de replay native Forgejo, pas sur le retry HTTP vers le receiver testé dans WEBHOOK-LIVE-EMITTED-018.

## Commande

```sh
PYTHONDONTWRITEBYTECODE=1 python3 scripts/run_forgejo_live_webhook_native_replay_e2e.py \
  --forgejo-binary /tmp/forgejo-17.0.0 \
  --repo-root /home/pc6/Documents/Codex/2026-10-21-mission-construire-une-forge-con-ue/.runtime/webhook-005-workspace \
  --state-dir /tmp/forgejo-native-replay-003-state \
  --evidence-dir evidence/webhook-native-replay-003
```

Le runner construit le receiver Rust depuis l’arbre de travail sous Bubblewrap avec registry staged, `--offline --locked`, namespace réseau et PID séparés et environnement vidé. L’instance Forgejo de test écoute sur `127.0.0.1:3130`; le receiver écoute sur un port loopback attribué. Les comptes, secret, DB SQLite et journaux d’exécution restent sous `/tmp/forgejo-native-replay-003-state` (permissions privées); cet état est conservé.

## Résultat observé

- Version binaire : Forgejo `17.0.0+gitea-1.22.0`, SHA-256 `e5ec437cd6357e8d1b4580cb47ab1daa58a32b33971a63db63446a535d33adb7`. Le code officiel audité est tag `v17.0.0`, commit `0d791421f85ca13a63b665053d81ea765fbbc571`; le lien de provenance entre ce commit source et ce binaire local n’est pas établi.
- Un événement d’issue `opened` est d’abord livré avec secret de receiver volontairement incorrect. Forgejo inscrit le premier task comme livré en échec HTTP 401; aucun événement n’est accepté dans le journal.
- Après une observation de 1 s, la table compte toujours un task. Le runner ne mesure pas le nombre de requêtes reçues par le receiver : cela n’établit pas l’absence de répétition HTTP du même task.
- POST sans cookie de session : HTTP 303 vers `/user/login`, sans task ajouté. POST authentifié mais cross-origin : HTTP 403, aucun task ajouté. POST authentifié avec un UUID inexistant : HTTP 404, aucun task ajouté.
- Deux POST de replay avec la même session admin et le même UUID initial échoué renvoient chacun HTTP 303. Forgejo crée deux tasks supplémentaires et deux UUID distincts; chaque payload conserve le même SHA-256 que l’original. Le receiver vérifie le HMAC et inscrit les deux événements `issues/opened` en 202, séquences 1 puis 2. Le replay répété n’est donc pas idempotent : UUID différents ne dédupliquent pas la même occurrence métier.
- Le readback confirme le hook inactif. Forgejo et les receivers suivis ont quitté; les listeners sur leurs ports sont absents.

Les détails machine lisibles sont dans `result.json`, `native-replay.json`, `hook-tasks.json`, `webhook-events.json`, `hook-readback.json` et la provenance de build. Le manifeste `SHA256SUMS` couvre les artefacts du dossier.

## Vérifications

- `PYTHONDONTWRITEBYTECODE=1 python3 -m unittest -v tests.test_forgejo_webhook_native_replay_e2e` : 5 tests passent. Ils lient un task échoué au payload inchangé, aux UUID nouveaux, aux séquences attendues et refusent les UUID/empreintes/séquences divergents.
- `PYTHONDONTWRITEBYTECODE=1 python3 -m py_compile scripts/run_forgejo_live_webhook_native_replay_e2e.py tests/test_forgejo_webhook_native_replay_e2e.py` passe avant l’exécution de bout en bout.
- Une première tentative 001 a échoué sur le harnais (mauvais module pour la vérification d’absence du listener), après arrêt du receiver et avant le replay web. Sa preuve d’échec reste sous `evidence/webhook-native-replay-001/`; elle n’est pas comptée comme succès. La tentative 002 a validé le replay simple; 003 étend la couverture aux refus sans session, cross-origin, UUID absent et double replay.

## Limites

Ce n’est pas encore le parcours demande → exigences → mission → agent → PR → vérificateur indépendant → merge. Aucun agent LLM, AgentOS, Forge Code, RAG ou mémoire n’a participé; ce run ne mesure ni coût en tokens ni valeur comparative. `n=1`, instance locale, compte admin de test; aucun compte non-admin n’a été testé. Le runner lit la table interne SQLite `hook_task` propre à Forgejo v17. Les services partagent l’UID hôte; Bubblewrap isole le build, pas l’exécution combinée Forgejo/receiver. Aucun test Go Forgejo ni association certaine du binaire au commit source n’est établi. Une signature valide atteste la requête locale et non la correction du logiciel.
