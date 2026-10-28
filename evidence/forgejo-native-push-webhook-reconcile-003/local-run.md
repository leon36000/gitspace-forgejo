# Batch réel de deux webhooks `push` vers une lecture Forgejo unique — 2026-10-25

## Résultat

Forgejo 17.0.0 local a reçu deux commits réels par Git HTTP sur `agent/native-push-proof`. Le premier push a créé la ref : livraison `0615a42b-b901-436e-bdef-d5c4187edd82`, `before=000…000`, `after=4672785eb15678e9dc8a9d5d53e2357100c5b4d7`. Le second l’a avancée : livraison `681a81fa-b84c-4ce5-965b-5ec13ab79fc7`, `before=4672785eb15678e9dc8a9d5d53e2357100c5b4d7`, `after=75e6f497e9d45c79504d790cde60e1c0ce8e6ffc`. Forgejo a enregistré une tâche `push` réussie et HTTP 202 pour chaque UUID.

Le runner a passé les deux UUIDs ensemble au CLI Rust de réconciliation. Le batch v5 retourne deux résultats `current_state_read` dans l’ordre demandé, correctement liés aux UUIDs, et indique `upstream_read_attempts=1` pour la branche commune. L’événement ancien annonce `4672785e…`, mais son résultat canonique pointe déjà sur `75e6f497…`; l’événement récent pointe sur la même tête. La lecture API séparée d’un compte non-admin sans droit `push` confirme ce SHA. Le journal n’a changé aucun octet pendant le batch. Le hook est désactivé puis relu; Forgejo et le récepteur se sont arrêtés sans listener.

Les sorties expurgées sont conservées dans [result.json](result.json), [push-events.json](push-events.json), [forgejo-hook-tasks.json](forgejo-hook-tasks.json), [branch-readback.json](branch-readback.json) et [reconciliation.json](reconciliation.json). Le credential reste sur un pipe privé hérité par le CLI; il n’est pas dans argv, l’environnement ni les artefacts.

## Sources et environnement

- Forgejo local `17.0.0+gitea-1.22.0`, binaire `/tmp/forgejo-17.0.0`, SHA-256 `e5ec437cd6357e8d1b4580cb47ab1daa58a32b33971a63db63446a535d33adb7`.
- HEAD produit compilé : `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`; worktree modifié.
- Python 3.12.3, Git 2.43.0, Cargo/rustc 1.95.0, Bubblewrap 0.9.0.
- Les deux binaires Rust ont été compilés `cargo build --offline --locked` dans Bubblewrap, avec namespaces réseau/PID séparés, environnement vidé et sources/registry/sysroot en lecture seule. Voir [cargo-build.log](cargo-build.log).
- Dépôt privé sur loopback `127.0.0.1:3130`; lecteur non-admin sans permission push; hook autorisé uniquement vers l’adresse loopback du récepteur jetable.

Commande E2E :

```text
python3 scripts/run_forgejo_native_push_webhook_reconcile.py --forgejo-binary /tmp/forgejo-17.0.0 --state-dir /tmp/forgejo-native-push-20261025-003 --evidence-dir evidence/forgejo-native-push-webhook-reconcile-003
```

## Vérifications

- `python3 -m unittest discover -s tests -p 'test_forgejo_emitted_webhook_e2e.py'` — 34/34, dont acceptation d’un batch ancien/courant, refus si le compteur d’accès dépasse un, refus d’UUID de résultat discordant.
- `python3 -m py_compile scripts/run_forgejo_live_webhook_reconcile_emitted.py scripts/run_forgejo_native_push_webhook_reconcile.py tests/test_forgejo_emitted_webhook_e2e.py` — code 0.
- `python3 scripts/verify_cargo_license_state.py` — vérifié; `license=null`, `license_file=null`, `publish=[]`, `no_license_selected`, `redistribution_rights=not_assessed`.
- `python3 -m unittest -v scripts/test_cargo_license_state.py` — 7/7.
- `git diff --check` — code 0.
- Sorties des commandes dans `unit-tests.log`, `py-compile.log`, `cargo-license-guard.log`, `cargo-license-tests.log` et `diff-check.log`; le manifeste local est vérifié après ajout du rapport.

## Limites

Un seul dépôt local et un seul batch de deux livraisons : cela ne mesure pas le débit/concurrence de plusieurs workers et ne démontre pas le comportement multi-tenant. Le compteur `upstream_read_attempts` est le compteur du CLI Rust, pas une capture indépendante de trafic. Aucun code candidat n’a été exécuté. Aucun LLM/AgentOS, PR/fusion ou benchmark conventionnel n’est inclus. Aucun code Forgejo n’a été copié ou modifié; aucune licence produit ou permission de redistribution n’est choisie.
