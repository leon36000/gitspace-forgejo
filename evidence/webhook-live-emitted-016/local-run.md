# WEBHOOK-LIVE-EMITTED-016 — redélivraison après reprise du receiver (24 octobre 2026)

## Reproduction

Le run utilise le worktree local modifié, HEAD de base `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`, et Forgejo `17.0.0+gitea-1.22.0`. Le binaire `/tmp/forgejo-17.0.0` a le SHA-256 `e5ec437cd6357e8d1b4580cb47ab1daa58a32b33971a63db63446a535d33adb7`; sa provenance de compilation exacte n’est pas établie.

```sh
PYTHONDONTWRITEBYTECODE=1 python3 -m unittest tests.test_forgejo_emitted_webhook_e2e -v
PYTHONDONTWRITEBYTECODE=1 python3 scripts/run_forgejo_live_webhook_reconcile_emitted.py \
  --forgejo-binary /tmp/forgejo-17.0.0 \
  --repo-root /home/pc6/Documents/Codex/2026-10-21-mission-construire-une-forge-con-ue/.runtime/webhook-005-workspace \
  --state-dir /tmp/forgejo-live-webhook-emitted-016-state \
  --evidence-dir /home/pc6/Documents/Codex/2026-10-21-mission-construire-une-forge-con-ue/.runtime/webhook-005-workspace/evidence/webhook-live-emitted-016
(cd evidence/webhook-live-emitted-016 && sha256sum -c SHA256SUMS)
```

Les 19 tests du runner passent; leur sortie est conservée dans [`python-tests.log`](python-tests.log). Cargo/Rust 1.95.0 a compilé les exécutables Rust sous Bubblewrap 0.9.0 avec environnement nettoyé, namespaces réseau et PID retirés, sources/registre/sysroot en lecture seule, puis `--offline --locked`. Les commandes, hashes des exécutables et hashes des sources sont dans [`provenance.json`](provenance.json) et [`cargo-build.log`](cargo-build.log).

## Résultat

Forgejo a réellement émis six tâches de webhook `issues` et `pull_request`, chacune reçue en HTTP 202 et liée à son UUID journalisé. Après ces livraisons, le receiver s’arrête proprement puis redémarre sur le même journal. Le runner renvoie le payload JSON de l’événement réel d’ouverture d’issue avec le même UUID et un HMAC calculé avec le secret local : le receiver répond HTTP 202, `duplicate=true`, avec le même UUID et la séquence 1. Le journal reste à six entrées avant et après; le readback est dans [`duplicate-delivery-replay.json`](duplicate-delivery-replay.json).

Les six IDs sont ensuite réconciliés contre l’API Forgejo courante. L’issue et la PR sont fermées, la PR reste non fusionnée; les résultats correspondent aux IDs demandés. Le credential invalide demeure `inconclusive/upstream_unauthorized`. Le hook est relu inactif, les deux instances du receiver et Forgejo sortent proprement, et les listeners sont absents. Détails : [`result.json`](result.json), [`webhook-events.json`](webhook-events.json), [`forgejo-hook-tasks.json`](forgejo-hook-tasks.json) et [`port-cleanup-readback.json`](port-cleanup-readback.json).

## Portée et limites

Cette preuve combine un événement initial réellement émis par Forgejo et une redélivraison locale après redémarrage. Le receiver conserve l’objet JSON décodé, pas les octets HTTP d’origine : le JSON de redélivraison est sérialisé de nouveau et signé à nouveau. Ce run ne prouve donc ni une répétition octet pour octet ni le mécanisme de retry natif de Forgejo. Le redémarrage est ordonné par SIGTERM; cette tranche ne simule pas SIGKILL à cette frontière.

Forgejo et le receiver restent des services locaux sous le même UID; le hook n’accepte que loopback et le code candidat n’est pas exécuté. Le binaire Forgejo n’a pas de provenance de build établie. `n=1`; aucun LLM, AgentOS, Forge Code, RAG ou mémoire n’a été utilisé, et aucun gain de qualité, coût ou délai n’est mesuré. L’état privé `/tmp/forgejo-live-webhook-emitted-016-state` est conservé.
