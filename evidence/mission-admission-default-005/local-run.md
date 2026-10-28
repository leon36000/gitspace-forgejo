# WEBHOOK-MISSION-ADMISSION-DEFAULT-001 — compatibilité du runner (24 octobre 2026)

Commande exécutée sans `--native-replay-admission` :

```sh
python3 scripts/run_forgejo_live_webhook_mission_admission.py \
  --forgejo-binary /tmp/forgejo-17.0.0 \
  --state-dir /tmp/forgejo-mission-admission-default-005 \
  --evidence-dir evidence/mission-admission-default-005 \
  --allocation-id mission012-default-admission-005
```

Forgejo v17.0.0+gitea-1.22.0 (SHA-256 `e5ec437cd6357e8d1b4580cb47ab1daa58a32b33971a63db63446a535d33adb7`) a écouté sur `127.0.0.1:3132`. HEAD source `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`; les sources sont non commitées dans un worktree déjà très sale.

Résultat : verdict `passed`, une livraison receiver, aucun replay natif, mauvais credential refusé avant claim, hook désactivé, processus arrêtés et listeners absents. `agent_runtime_invoked=false`; ce run confirme la compatibilité du chemin par défaut, pas l’exécution d’un agent. Le JSON et sa provenance sont dans [result.json](result.json) et [provenance.json](provenance.json); l’état privé est conservé sous `/tmp/forgejo-mission-admission-default-005`.

Ce run `n=1` ne prouve pas de gain de productivité ni le parcours agent→PR→vérification→merge. La construction Forgejo/binaire n’est pas reliée au commit source audité.
