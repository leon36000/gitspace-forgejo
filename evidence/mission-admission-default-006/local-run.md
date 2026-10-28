# WEBHOOK-MISSION-ADMISSION-DEFAULT-001 — compatibilité finale (24 octobre 2026)

Commande exécutée sans `--native-replay-admission` :

```sh
python3 scripts/run_forgejo_live_webhook_mission_admission.py \
  --forgejo-binary /tmp/forgejo-17.0.0 \
  --state-dir /tmp/forgejo-mission-admission-default-006 \
  --evidence-dir evidence/mission-admission-default-006 \
  --allocation-id mission012-default-admission-006
```

Forgejo v17.0.0+gitea-1.22.0 (SHA-256 `e5ec437cd6357e8d1b4580cb47ab1daa58a32b33971a63db63446a535d33adb7`) a écouté sur `127.0.0.1:3132`; HEAD source `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`, worktree très sale et sources non commitées.

Résultat : verdict `passed`, une seule livraison receiver, aucun replay natif; credential invalide refusé avant claim; hook désactivé, processus arrêtés, listeners absents. `agent_runtime_invoked=false`; ce run confirme le chemin par défaut après le binding complet du payload, pas l’exécution d’un agent. Reçus et provenance : [result.json](result.json), [provenance.json](provenance.json). État privé sous `/tmp/forgejo-mission-admission-default-006`.

Limites : run `n=1`, aucune PR, vérification de candidat, merge ou comparaison de valeur. Le binaire Forgejo local n’est pas relié au commit source audité.
