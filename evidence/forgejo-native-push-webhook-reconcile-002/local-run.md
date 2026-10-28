# Push natif puis mise à jour de branche : réconciliation fraîche — 2026-10-25

## Résultat observé

Forgejo 17.0.0 local a reçu deux pushes Git HTTP réels vers `agent/native-push-proof`. Le premier a créé la ref (`before=000…000`, `after=783f641e0a41108354975d14a06eed5a949a05fd`). Le second l’a avancée (`before=783f641e0a41108354975d14a06eed5a949a05fd`, `after=90af296ab19bc50ddaa9356bdae9f907a73f09f6`). Forgejo a émis deux événements `push` avec UUID distincts, deux tâches `push` réussies et deux réponses HTTP 202; le journal conserve les deux séquences dans l’ordre.

Après le second push, le vrai CLI de réconciliation a lu l’état Forgejo pour chacune des deux livraisons. Pour l’ancienne livraison, le `notification_after_sha` est encore `783f…`, mais le résultat canonique est déjà la tête courante `90af…`. La livraison récente retourne la même tête courante. Chaque résultat a `verdict=current_state_read`, `upstream_read_attempts=1`, le bon UUID de livraison et l’URL avec `/` encodé. L’API Forgejo confirme le même SHA pour la branche. Le journal est inchangé octet par octet après les deux lectures. Le hook est désactivé et les listeners Forgejo/récepteur sont absents après arrêt.

Les objets expurgés sont dans [result.json](result.json), [push-events.json](push-events.json), [forgejo-hook-tasks.json](forgejo-hook-tasks.json), [branch-readback.json](branch-readback.json) et [reconciliation.json](reconciliation.json). Les jetons ne sont ni conservés ni inclus dans les sorties.

## Exécution et versions

- Dépôt produit compilé : HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`; le worktree était modifié.
- Forgejo `17.0.0+gitea-1.22.0`, binaire local `/tmp/forgejo-17.0.0`, SHA-256 `e5ec437cd6357e8d1b4580cb47ab1daa58a32b33971a63db63446a535d33adb7`.
- Python 3.12.3, Git 2.43.0, Cargo/rustc 1.95.0, Bubblewrap 0.9.0.
- Les binaires receiver et reconciler ont été compilés `--offline --locked` dans Bubblewrap : namespaces réseau et PID séparés, environnement vidé, sources/dépendances montées en lecture seule. Voir [cargo-build.log](cargo-build.log).
- L’instance Forgejo était privée, liée à `127.0.0.1:3130`; l’identité de lecture était non-admin et n’avait pas le droit `push`. L’allowlist du webhook autorisait seulement `127.0.0.1` dans cette instance jetable.

Commande E2E :

```text
python3 scripts/run_forgejo_native_push_webhook_reconcile.py --forgejo-binary /tmp/forgejo-17.0.0 --state-dir /tmp/forgejo-native-push-20261025-002 --evidence-dir evidence/forgejo-native-push-webhook-reconcile-002
```

## Vérifications

- `python3 -m unittest discover -s tests -p 'test_forgejo_emitted_webhook_e2e.py'` — 31/31, dont création, mise à jour avec SHA `before` non nul, SHA invalide, scope de dépôt, événement et tâche Forgejo.
- Test-first observé avant la correction : `python3 -m unittest discover -s tests -p 'test_forgejo_emitted_webhook_e2e.py' -k accepts_update_to_existing_branch` échoue sur le parseur initial, avec `ValueError: webhook push is not a valid new-branch update`; le même ciblé passe ensuite avec le parseur généralisé. Cette observation n’est pas rejouable contre le code final car le comportement attendu est maintenant implémenté.
- `python3 -m py_compile scripts/run_forgejo_live_webhook_reconcile_emitted.py scripts/run_forgejo_native_push_webhook_reconcile.py tests/test_forgejo_emitted_webhook_e2e.py` — code 0.
- `python3 scripts/verify_cargo_license_state.py` — `status=verified`, `license=null`, `license_file=null`, `publish=[]`, `no_license_selected`, droits `not_assessed`.
- `python3 -m unittest -v scripts/test_cargo_license_state.py` — 7/7.
- `git diff --check` — code 0.
- Les sorties détaillées sont conservées dans `unit-tests.log`, `py-compile.log`, `cargo-license-guard.log`, `cargo-license-tests.log` et `diff-check.log`; le manifeste local est vérifié après l’ajout du rapport.

## Portée et limites

La preuve couvre deux émissions natives Forgejo et le comportement du reconciler face à un événement push devenu ancien. Elle établit la lecture de la tête courante pour ces livraisons précises; ce n’est ni un stress concurrent ni une preuve générale pour tous les writers/serveurs. Aucun code du dépôt candidat n’a été exécuté. Cette tranche ne couvre pas un agent LLM/AgentOS, la revue PR/fusion ou un benchmark conventionnel. Aucun code Forgejo n’a été copié/modifié; aucune licence produit n’a été choisie et aucun droit de redistribution n’a été inféré.
