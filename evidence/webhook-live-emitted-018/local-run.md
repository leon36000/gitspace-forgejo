# WEBHOOK-LIVE-EMITTED-018 — arrêt brutal et redélivrance après reprise

Date : 24 octobre 2026. Exécution locale isolée contre Forgejo v17, sur le worktree non commité `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`.

## Parcours et résultat

Le runner a créé six événements réels `issues`/`pull_request` dans Forgejo et les tâches du hook ont chacune reçu HTTP 202. Il a ensuite arrêté le processus receiver avec `SIGKILL` après les six append, constaté l’absence de listener, puis relancé le même binaire sur le même journal. La redélivrance du premier événement a reçu HTTP 202 avec `duplicate=true`, le même UUID, la séquence initiale 1 et `reconcile_required=true`.

Le journal avait 6 entrées et 41 361 octets avant et après cette reprise; son SHA-256 est resté `67a26248531d42d30f0e6c5eceed88fa2fa86b139da95fba811089554af6003d`. Les six notifications ont été rapprochées de l’état courant Forgejo. Le lecteur API avait les permissions `pull` sans `push` ni `admin`; un credential invalide a retourné `inconclusive/upstream_unauthorized`. Le hook a été relu inactif avant l’arrêt de Forgejo; tous les processus suivis sont sortis et aucun listener de test ne subsiste.

La sortie structurée est dans [`result.json`](result.json), les six tâches lues dans [`forgejo-hook-tasks.json`](forgejo-hook-tasks.json), le doublon dans [`duplicate-delivery-replay.json`](duplicate-delivery-replay.json), le refus d’authentification dans [`invalid-credential-result.json`](invalid-credential-result.json), et les readbacks d’arrêt dans [`port-cleanup-readback.json`](port-cleanup-readback.json). La provenance et les empreintes source/build sont dans [`provenance.json`](provenance.json) et [`cargo-build.log`](cargo-build.log).

## Reproduction et vérifications

Depuis la racine du worktree :

```sh
PYTHONDONTWRITEBYTECODE=1 python3 scripts/run_forgejo_live_webhook_reconcile_emitted.py \
  --forgejo-binary /tmp/forgejo-17.0.0 \
  --repo-root "$PWD" \
  --state-dir /tmp/forgejo-live-webhook-emitted-018-state \
  --evidence-dir "$PWD/evidence/webhook-live-emitted-018"
PYTHONDONTWRITEBYTECODE=1 python3 tests/test_forgejo_emitted_webhook_e2e.py
sha256sum -c evidence/webhook-live-emitted-018/SHA256SUMS
```

Le test ciblé passe 22 tests (`python-tests.log`). Le run rapporte Forgejo `17.0.0+gitea-1.22.0`, binaire SHA-256 `e5ec437cd6357e8d1b4580cb47ab1daa58a32b33971a63db63446a535d33adb7`; le build local a utilisé Cargo/rustc 1.95.0, Bubblewrap 0.9.0 et `--offline --locked`. Le commit exact de l’exécutable Forgejo n’est pas établi par la provenance disponible. L’exécutable Rust du receiver/réconciliateur a été construit depuis un arbre source staged dont l’empreinte et les SHA individuels sont enregistrés dans `provenance.json`.

SHA-256 du runner : `360de019cceeaedbf0b18c9b524cbb46ccd10aba1a7b285ee4765a491a4af473`; tests : `4d0beaff56688d931ab5602245626db2ca480c01683ba1de2e38aadc9c675c89`.

## Portée et limites

Forgejo a réellement émis les livraisons initiales; la redélivrance est un POST local créé depuis l’objet JSON décodé du journal et re-signé avec le secret synthétique. Le receiver ne conserve pas les octets HTTP originaux : ce scénario ne teste ni le retry natif Forgejo ni une répétition octet pour octet. Le `SIGKILL` a lieu après les append acceptés et avant le replay; il ne simule pas une interruption au milieu d’un append, ni un crash Forgejo. Le run est `n=1`.

Cette tranche vérifie le runner et son adaptation à la reprise du receiver; elle n’exécute pas un agent LLM et n’intègre pas AgentOS, Forge Code, RAG ou mémoire. Elle ne mesure pas le gain produit. Forgejo et receiver partagent l’UID hôte; Bubblewrap isole le build seulement, pas les services vivants. Le worktree source était sale et les modifications restent locales et non commitées. L’état privé du run reste dans `/tmp/forgejo-live-webhook-emitted-018-state`.
