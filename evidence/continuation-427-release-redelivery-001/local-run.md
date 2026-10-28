# CONTINUATION-427 — Rediffusion Release après redémarrage

Date : 2026-10-28 UTC. Worktree `/home/pc6/Documents/Codex/2026-10-21-mission-construire-une-forge-con-ue/.runtime/webhook-005-workspace`, branche `local/WEBHOOK-005-canonical-links`, HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. État Git déjà fortement modifié; les nouveaux artefacts restent locaux, sans staging/commit/push.

## Changement

Le runner Release introduit dans CONTINUATION-426 choisit maintenant son propre delivery pour la rediffusion opt-in après SIGKILL/redémarrage du receiver. `select_duplicate_replay_event` reçoit facultativement `release_event`, le valide comme release/action connue et le préfère au scénario PR; si absent, le chemin historique PR ou issue est inchangé. Le runner extrait ensuite de l’enregistrement vérifié son payload JSON, re-signe le POST vers l’adresse loopback, puis réutilise les validations existantes du statut `202`, delivery ID, séquence, indicateur duplicate et journal inchangé.

Tests ajoutés : la sélection choisit bien le Release pour le mode opt-in; le sender envoie `X-Forgejo-Event: release` et une signature HMAC correcte sur le payload capturé. Le TDD a échoué sur le sélecteur avant changement (`unexpected keyword argument 'release_event'`); le sender release passait déjà, car le type `release` faisait partie de l’allowlist générique.

## Vérifications

- `PYTHONPATH=tests python3 -m unittest test_forgejo_emitted_webhook_e2e.DuplicateDeliveryReplayTests` — **6/6 réussis**.
- `PYTHONPATH=tests python3 -m unittest discover -s tests -p 'test_forgejo_emitted_webhook_e2e.py'` — **186/186 réussis**.
- `python3 scripts/test_cargo_license_state.py` — **12/12 réussis**.
- `python3 scripts/verify_cargo_license_state.py` — `metadata_policy_verified`; `license=null`, `license_file=null`, `publish=[]`, `no_license_selected`, droits `not_assessed`, marqueur `NOASSERTION`.
- `python3 -m py_compile scripts/run_forgejo_live_webhook_reconcile_emitted.py tests/test_forgejo_emitted_webhook_e2e.py`, `git diff --check` et `--help` pour `--release-webhook` — réussis.

Commandes et environnement : `commands.txt`, `environment.txt`.

## Limites et suite

Les tests exécutés par ce lot sont des tests unitaires/fixtures. Ils ne lancent pas le receiver Rust ni Forgejo : l’API loopback dans le test sender confirme l’en-tête et le HMAC, sans prouver le refus duplicate du receiver réel. Le test Rust générique existant couvre l’idempotence à d’autres niveaux, mais aucune rediffusion d’une Release émise n’a été exécutée dans ce lot. `forgejo` et `go` restent absents du PATH; `/` ne dispose que d’environ 408 MB, et le runner fixe son staging sous `/tmp`. Aucun appel modèle/AgentOS, changement Cargo/Rust, contact externe ou écriture GitLab n’a eu lieu.

Lancer ensuite l’option `--release-webhook` dans une instance Forgejo v17 jetable, confirmer la release native, sa rediffusion après SIGKILL, le journal byte-identique et le readback CLI. Conserver séparément les résultats fixture et live; la mission entière et le benchmark restent incomplets.
