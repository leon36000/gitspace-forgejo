# WEBHOOK-026/027 — credential refusé et conflit concurrent au receiver

Date : 23 octobre 2026. Worktree observé à `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`; le worktree était déjà très modifié. Aucun commit, push ou changement de production.

## Changements

- WEBHOOK-026 ajoute un scénario qui journalise une livraison signée par le vrai receiver, lance le vrai binaire `forgejo-reconcile-local` avec un credential synthétique invalide passé par descripteur, puis exige un HTTP 401, un verdict structuré `inconclusive/upstream_unauthorized`, aucun retry et aucune fuite du token dans stdout/stderr.
- WEBHOOK-027 ajoute un test de processus du vrai `forgejo-webhook-local`. Deux clients libérés ensemble envoient le même ID avec des corps signés différents. Un seul corps est accepté (`202`, `duplicate=false`), l’autre est refusé (`409`, `delivery_conflict`). Le journal rejoué contient exactement le corps gagnant. Après redémarrage, le gagnant retourne un doublon, le corps perdant reste conflictuel et le journal ne reçoit aucun nouvel événement.
- Le fichier de test est formaté en Rust 2024; les trois écarts rustfmt précédemment présents dans son scénario PR sont corrigés.

## Vérifications

- `python3 scripts/run_webhook_reconcile_cli_e2e.py` : 7 scénarios de processus passent sous `bwrap --unshare-net`, dont credentials valides et invalides, upstream interdit, retries bornés et reprise après interruption.
- `cargo test --offline --locked --all-targets -- --test-threads=4` : 32 groupes, 361 réussis, 0 échec, 14 ignorés. Le nouveau conflit concurrent est exécuté dans cette suite; son exécution ciblée passe aussi.
- `cargo clippy --offline --locked --all-targets -- -A clippy::too_many_arguments -A clippy::bool_assert_comparison -A clippy::manual_div_ceil` : exit 0 avec ces allowances explicites.
- `rustfmt --edition 2024 --check src/bin/forgejo-reconcile-local.rs tests/webhook_receiver_process_e2e.rs` : exit 0.
- `git diff --check` sur les fichiers du lot : exit 0.

Sorties reproductibles : `webhook-026-process-e2e-final.log`, `webhook-027-concurrent-conflict-targeted.log`, `webhook-027-all-targets-final.log`, `webhook-027-clippy-final.log`, `webhook-027-rustfmt-final.log` et `webhook-027-sha256.txt`.

## Couverture et limites

La barrière relâche les clients ensemble, mais ne prouve pas que les deux workers du receiver atteignent simultanément le verrou d’append; ce test n’est pas une course déterministe au verrou ni un stress test. Le contrat `webhook_contract::concurrent_retries_append_exactly_once` couvre déjà les append concurrents au niveau ingestion. Le service Forgejo du scénario CLI est une fixture synthétique; cette tranche n’intègre ni Forgejo réel, ni AgentOS, ni RAG, ni moteur de mémoire, ni LLM. Le parcours Forgejo v17 live précédent demeure décrit séparément dans [WEBHOOK-025](webhook-025-local-run.md). Ces tests ne démontrent pas d’amélioration statistique, d’isolation contre un processus hostile du même UID, ni de déploiement réel.
