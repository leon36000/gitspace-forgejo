# WEBHOOK-LIVE-EMITTED-017 — intégrité octet du journal après redélivraison (24 octobre 2026)

## Reproduction

Run depuis le worktree local modifié, sur HEAD de base `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. Forgejo est `17.0.0+gitea-1.22.0`; son binaire `/tmp/forgejo-17.0.0` a le SHA-256 `e5ec437cd6357e8d1b4580cb47ab1daa58a32b33971a63db63446a535d33adb7`. La provenance de compilation de ce binaire n’est pas établie.

```sh
PYTHONDONTWRITEBYTECODE=1 python3 -m unittest tests.test_forgejo_emitted_webhook_e2e -v
PYTHONDONTWRITEBYTECODE=1 python3 scripts/run_forgejo_live_webhook_reconcile_emitted.py \
  --forgejo-binary /tmp/forgejo-17.0.0 \
  --repo-root /home/pc6/Documents/Codex/2026-10-21-mission-construire-une-forge-con-ue/.runtime/webhook-005-workspace \
  --state-dir /tmp/forgejo-live-webhook-emitted-017-state \
  --evidence-dir /home/pc6/Documents/Codex/2026-10-21-mission-construire-une-forge-con-ue/.runtime/webhook-005-workspace/evidence/webhook-live-emitted-017
(cd evidence/webhook-live-emitted-017 && sha256sum -c SHA256SUMS)
```

Les 20 tests ciblés passent; la sortie est conservée dans [`python-tests.log`](python-tests.log). Cargo/Rust 1.95.0 construit les binaires sous Bubblewrap 0.9.0 avec environnement nettoyé, namespaces réseau et PID retirés, sources/registre/sysroot en lecture seule, puis `--offline --locked`. La provenance détaillée est dans [`provenance.json`](provenance.json), les commandes dans [`cargo-build.log`](cargo-build.log), et le reçu complet dans [`result.json`](result.json).

## Résultat

Forgejo a réellement émis six tâches `issues` et `pull_request`, toutes reçues en HTTP 202 et reliées aux UUID de l’inbox. Le receiver s’arrête proprement et redémarre avec le même fichier journal. Le runner renvoie l’objet JSON d’ouverture d’issue provenant de l’événement Forgejo, avec le même UUID et un HMAC produit à partir du secret jetable. Le receiver répond HTTP 202, `duplicate=true`, UUID identique, séquence 1. Le journal contient toujours six entrées; ses octets sont identiques avant et après : taille 41 361, SHA-256 `b0db7b2b0eeb14c3829efc37ed48b1d68da1c600a42f32aa9b1c56421884aab7`. Le reçu dédié est [`duplicate-delivery-replay.json`](duplicate-delivery-replay.json).

Les six événements sont réconciliés à l’état courant Forgejo; l’issue et la PR sont fermées, la PR non fusionnée. Le credential invalide reste `inconclusive/upstream_unauthorized`. Le hook est relu inactif et les listeners sont absents après les deux arrêts du receiver et l’arrêt de Forgejo. Voir [`webhook-events.json`](webhook-events.json), [`forgejo-hook-tasks.json`](forgejo-hook-tasks.json) et [`port-cleanup-readback.json`](port-cleanup-readback.json).

## Portée et limites

La revue ciblée a motivé deux renforcements : comparer aussi les octets du journal, pas uniquement les objets JSON, et tester le rejet d’une séquence erronée. Ces refus sont protégés par les tests du runner et la validation de bytes est exercée pendant le run 017.

La première livraison vient de Forgejo; le renvoi est local, depuis l’objet JSON décodé du journal, puis sérialisé et signé à nouveau. Le receiver ne conserve pas les octets HTTP d’origine : ce n’est ni une répétition octet pour octet du corps, ni le retry natif de Forgejo. Le redémarrage est ordonné par SIGTERM, pas une panne SIGKILL. Forgejo et receiver partagent l’UID hôte; le hook accepte loopback uniquement et le code candidat n’est pas exécuté. Binaire Forgejo sans provenance de build établie, `n=1`, aucun LLM, AgentOS, Forge Code, RAG ou mémoire et aucun gain mesuré. L’état privé `/tmp/forgejo-live-webhook-emitted-017-state` est conservé.
