# WEBHOOK-006 — attente bornée du verrou inbox

## Résultat

`EventJournal::FileLock` acquiert maintenant le verrou avec `flock(LOCK_EX | LOCK_NB)` et une échéance monotone de 5 000 ms. Il conserve un premier essai immédiat pour les opérations non bloquantes, réessaie avec un sommeil maximal de 10 ms, puis renvoie `JournalError::LockBusy`. Le contrôle d’échéance précède chaque reprise : un processus désordonnancé ne peut pas obtenir le verrou après l’échéance uniquement parce qu’il vient d’être libéré.

`forgejo-reconcile-local --capabilities` déclare `wait=bounded` et `wait_timeout_ms=5000` pour `reconcile_webhook_deliveries`. Le CLI renvoie `inbox journal is busy; retry reconciliation after the active journal operation completes`, au lieu de rester bloqué indéfiniment.

## Contrat et vérifications

Les tests de `tests/reconcile_journal_lock_timeout.rs` lancent le binaire réel :

- la découverte des capacités annonce la borne de 5 000 ms;
- avec le verrou toujours détenu, le CLI sort avant 6 secondes, après au moins 4,5 secondes, code 1, avec l’erreur actionnable;
- si le verrou est libéré après 250 ms, le CLI reprend, cherche la livraison et renvoie l’erreur métier « delivery ID … was not found » plutôt que `LockBusy`.

Commandes finales sous Rust 1.95.0 / Cargo 1.95.0, Linux aarch64 :

| Commande | Résultat |
|---|---|
| `cargo test --offline --locked --test reconcile_journal_lock_timeout -- --nocapture` | 3 réussis, 0 échec, 0 ignoré |
| `cargo test --offline --locked --all-targets` | 302 réussis, 0 échec, 12 ignorés sur 31 exécutables de tests |
| `cargo clippy --offline --locked --all-targets -- -D warnings -A clippy::too_many_arguments -A clippy::bool_assert_comparison -A clippy::manual_div_ceil` | succès avec les trois allowances historiques |
| `cargo build --offline --locked --release --all-targets` | succès |
| `rustfmt --edition 2024 --config skip_children=true --check src/lib.rs src/bin/forgejo-reconcile-local.rs tests/reconcile_journal_lock_timeout.rs` | succès |
| `git diff --check` | succès |

Le premier `cargo test --all-targets` de cette ronde a échoué une fois dans le test préexistant `allocation_replay_validates_later_records_for_the_same_mission`, au `create_dir` de `ScratchDir::new` avec `AlreadyExists` (code 17). Le helper construit son nom temporaire depuis PID et horloge; une collision entre créations parallèles est une explication plausible, mais reste une inférence. Le binaire de tests concerné a repassé seul (33 réussis, 0 échec, 2 ignorés), puis la suite complète a repassé. Le helper n’a pas été modifié et cet incident reste visible dans les journaux.

Les sorties brutes sont dans `journal-lock-timeout-001-focused-final.log`, `journal-lock-timeout-001-all-targets-final.log` (premier échec global), `journal-lock-timeout-001-existing-target-recheck.log`, `journal-lock-timeout-001-all-targets-rerun.log`, `journal-lock-timeout-001-clippy-final.log`, `journal-lock-timeout-001-release-build-final.log`, `journal-lock-timeout-001-rustfmt-final.log`, `journal-lock-timeout-001-diff-check-final.log` et `journal-lock-timeout-001-whitespace-check.log`.

## Identité et limites

HEAD au début et à la fin : `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`; le travail reste non commité dans le worktree local déjà modifié. Les changements principaux sont `src/lib.rs`, `src/bin/forgejo-reconcile-local.rs` et le nouveau test de processus. Les empreintes de ces fichiers et des résultats finaux se trouvent dans `journal-lock-timeout-001.sha256`.

Le délai ne couvre que la contention `flock`. Ouverture, lecture, écriture et synchronisation du système de fichiers ne sont pas bornées par ce mécanisme. L’inbox reste sans limite de taille et le verrou n’isole pas des processus de même UID. Le test ne contacte pas Forgejo et ne démontre pas un parcours LLM.
