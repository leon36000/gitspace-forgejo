# SANDBOX-CGROUP-LIMITS-001 — test cgroup activé

Date : 24 octobre 2026. Worktree `local/WEBHOOK-005-canonical-links`, base examinée `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`.

## Changement

`run_sandbox_test` crée déjà un scope `systemd-run --user --scope` avec `MemoryMax=1 073 741 824`, `TasksMax=64`, `CPUQuota=200%` et `RuntimeMaxSec`. Bubblewrap monte aussi `/target` (256 MiB) et `/tmp` (128 MiB) en tmpfs. Le test d’intégration `verifier_scope_enforces_memory_task_and_cpu_limits` vérifiait ces valeurs dans le cgroup courant et contrôlait l’arrêt d’un enfant détaché à l’échéance, mais restait ignoré.

Sur cet hôte, son exécution explicite sous `--ignored` a réussi. Le test a donc été retiré de la liste des ignorés et renommé `verifier_scope_applies_cgroup_limits_and_kills_detached_children_at_deadline` pour décrire la preuve observée. Aucun code de production ni politique de ressources n’a changé.

## Vérifications

- Test ciblé normal : `cargo test --locked --offline --bin mission-012-mcp-host host::tests::verifier_scope_applies_cgroup_limits_and_kills_detached_children_at_deadline -- --exact --nocapture` — 1 réussi ([journal](sandbox-cgroup-limits-001-focused.log)).
- Suite : `cargo test --locked --offline --all-targets -- --test-threads=4` — 34 groupes, 426 réussis, 0 échec, 22 ignorés ([journal complet](sandbox-cgroup-limits-001-all-targets.log)).
- Clippy : `cargo clippy --locked --offline --all-targets` — code de sortie 0, avec les avertissements préexistants de complexité, `collapsible_if`, comparaison booléenne, nommage d’enum et `div_ceil` ([journal](sandbox-cgroup-limits-001-clippy.log)).
- `cargo fmt --all -- --check` et `git diff --check` — succès.

Hôte : Linux `7.0.0-1019-nvidia`, aarch64; Rust/Cargo 1.95.0; systemd 255; Bubblewrap 0.9.0; cgroup v2 avec contrôleurs `cpu`, `memory` et `pids`. Les assertions du test confirment `memory.max=1073741824`, `pids.max=64` et `cpu.max=200000 100000`. Le second scope a interrompu le processus `setsid sleep` avant son délai de 30 secondes, puis le test a confirmé l’absence de son PID.

## Limites

Le test démontre la configuration effective des contrôleurs et l’arrêt d’un descendant à l’échéance; il ne pousse pas la mémoire ou le nombre de tâches jusqu’au seuil et ne mesure pas le CPU consommé. Il dépend d’un gestionnaire systemd utilisateur et d’un cgroup v2 inscriptible. La preuve ne certifie pas la sécurité générale du sandbox. Aucun Forgejo live, modèle, réseau externe, secret ou dépense n’a été utilisé.

SHA-256 du code vérifié `src/mission_012_host.rs` : `312d9bb7eb135fc9b6bb59c616d47451fbd11f98c9668a192a09f3af9dc5f7c4`.
