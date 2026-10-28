# Forgejo native push webhook reconciliation — 2026-10-25

## Résultat

Un dépôt privé Forgejo 17.0.0 jetable a reçu, par Git HTTP, un commit sur une nouvelle branche `agent/native-push-proof`. Le hook `push` natif a livré son événement signé au vrai processus `forgejo-webhook-local`; le journal a identifié `refs/heads/agent/native-push-proof`, `before=000…000` et `after=e9705e83258a08925f10de8931bd825877331dc3`. La tâche Forgejo associée porte le même UUID et a retourné HTTP 202.

Un compte lecteur non administrateur, sans droit `push`, a relu la branche dans Forgejo. Le vrai `forgejo-reconcile-local` a traité l’identifiant de livraison avec une lecture API, verdict `current_state_read`, une tentative upstream et le SHA de tête identique au commit poussé. L’URL de branche dans le résultat encode `/` (`agent%2Fnative-push-proof`). Le journal append-only est identique octet par octet avant et après la réconciliation. Le hook a été désactivé puis relu; Forgejo et le récepteur se sont arrêtés et aucun listener attendu ne subsistait.

Résultat et données expurgées : [result.json](result.json), [push-event.json](push-event.json), [forgejo-hook-tasks.json](forgejo-hook-tasks.json), [branch-readback.json](branch-readback.json), [reconciliation.json](reconciliation.json). Aucun secret, jeton ou corps d’authentification n’est enregistré.

## Sources et environnement

- Forgejo officiel local v17.0.0, `forgejo version 17.0.0+gitea-1.22.0`, binaire `/tmp/forgejo-17.0.0`, SHA-256 `e5ec437cd6357e8d1b4580cb47ab1daa58a32b33971a63db63446a535d33adb7`.
- HEAD du dépôt produit utilisé pour compiler : `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`; worktree local modifié.
- Python 3.12.3, Git 2.43.0, Cargo/rustc 1.95.0, Bubblewrap 0.9.0.
- Les deux CLIs Rust ont été compilés avec `cargo build --offline --locked` dans Bubblewrap, réseau et PID non partagés, environnement vidé et sources/dépendances en lecture seule. Le log et les hashes d’outils sont conservés dans [cargo-build.log](cargo-build.log) et `result.json`.
- Le test n’a contacté qu’une instance Forgejo locale sur `127.0.0.1:3130` et le récepteur sur loopback. L’allowlist de webhook `127.0.0.1` ne concerne que cette instance jetable.

## Commandes et vérification

- `python3 -m py_compile scripts/run_forgejo_live_webhook_reconcile_emitted.py scripts/run_forgejo_native_push_webhook_reconcile.py tests/test_forgejo_emitted_webhook_e2e.py` — réussi.
- `python3 -m unittest discover -s tests -p 'test_forgejo_emitted_webhook_e2e.py'` — 29 tests réussis.
- `python3 scripts/run_forgejo_native_push_webhook_reconcile.py --forgejo-binary /tmp/forgejo-17.0.0 --state-dir /tmp/forgejo-native-push-20261025-001 --evidence-dir evidence/forgejo-native-push-webhook-reconcile-001` — réussi; les sorties structurées sont conservées dans ce répertoire. L’enveloppe console redirigée est à côté sous `evidence/forgejo-native-push-webhook-reconcile-001-console.json`.
- `git diff --check` — réussi.
- Les sorties correspondantes sont conservées dans [unit-tests.log](unit-tests.log), [py-compile.log](py-compile.log) et [diff-check.log](diff-check.log). Le manifeste de ce répertoire est vérifié après ajout des sorties.

## Portée et limites

Cette preuve démontre un événement `push` natif de Forgejo v17, un transfert Git réel vers une nouvelle ref, l’association de tâche par UUID et la réconciliation live contre l’API avec un lecteur distinct. La branche de test contient un petit fichier de preuve; aucune commande du dépôt candidat n’a été exécutée. Ce scénario ne rejoue pas les parcours PR/fusion établis séparément.

Il ne démontre pas l’utilisation d’un LLM/AgentOS, une isolation multi-locataire en production, la coordination de writers externes, un gain de productivité, ni une comparaison Forgejo/CI conventionnels. Le code de Forgejo n’a pas été copié ou modifié. Aucun modèle, service externe, coût, publication, licence ou droit de redistribution nouveau n’a été utilisé ou choisi.
