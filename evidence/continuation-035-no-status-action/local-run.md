# CONTINUATION-035 — guidance actionnable quand le snapshot PR ne porte pas de preuve

Date : 26 octobre 2026. Workspace : `.runtime/webhook-005-workspace`. HEAD observé : `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`; worktree déjà fortement modifié. Les changements restent dans cet environnement local, aucun commit n’a été créé.

## Lacune et correction

`--pull-request-status-snapshot` relisait correctement la PR et rendait `observation=no_statuses` quand le SHA courant ne portait aucun statut Forgejo, mais l’enveloppe externe ne donnait ni raison ni action. La sortie a maintenant `reason=no_statuses` et demande un vérificateur indépendant; un résultat de pagination partiel (`statuses_complete=false`) retourne `reason=status_snapshot_incomplete` et demande de relire les pages sans le considérer comme succès. `--capabilities` expose `possible_observations=[complete,no_statuses,inconclusive]`, la politique du cas vide et la règle du cas incomplet.

Le test unitaire rouge `red.log` reproduit `reason=null` avant le changement. L’observation sans statuts termine avec code 0 parce que la lecture HTTP et les deux lectures PR sont complètes; le JSON contient `no_statuses`, `independent_verification=false`, et l’action exige une vérification indépendante. Le code de sortie décrit une lecture achevée, pas un verdict.

## Vérifications

Toolchain : Rust `rustc 1.95.0 (59807616e 2026-04-14)`, Cargo `1.95.0 (f2d3ce0bd 2026-03-21)`, Bubblewrap `0.9.0`, cible `aarch64-unknown-linux-gnu`; voir `tool-versions.txt`.

- `cargo test --locked --offline --bin forgejo-reconcile-local tests::pull_request_status_snapshot_ -- --test-threads=1` : 4 réussis, 0 échec, 134 filtrés. Couverture : tête stable, dérive de tête, absence de statuts, 500 contextes retournés sur 501 disponibles (pagination incomplète).
- `cargo test --locked --offline --test forgejo_reconcile_capabilities_e2e actual_cli_discloses_versioned_local_test_capabilities -- --exact --test-threads=1` : 1 réussi; le premier run du contrat est rouge parce que les observations/politiques manquent dans les capacités (`capabilities-red.log`), puis vert après ajout.
- Invocation processus sous `bwrap --unshare-net --unshare-pid --new-session` de `actual_cli_binds_status_snapshot_to_pr_head_and_refuses_head_drift --exact --ignored --nocapture --test-threads=1` : 1 réussi. Le vrai processus CLI passe les cas tête stable, dérive, et zéro statut; chaque cas reçoit exactement PR → statuts → PR. Zéro statut produit la guidance et ne devient pas un verdict `success`.
- `cargo test --locked --offline --test forgejo_reconcile_capabilities_e2e -- --test-threads=1` : 3 réussis, 0 échec, 2 ignorés par défaut (les deux tests qui réservent le port fixe 3130). Le cas E2E concerné a été lancé explicitement dans le namespace réseau isolé.
- `cargo clippy --locked --offline --bin forgejo-reconcile-local -- -D warnings` : échec avec 12 diagnostics répartis dans des modules hors de cette tranche; voir `clippy-strict.log`. Les catégories sont `too_many_arguments`, `collapsible_if`, `explicit_auto_deref`, `if_same_then_else` dans `src/agent_runtime.rs`, `src/forgejo.rs`, `src/mission_admission.rs`, `src/webhook/reconcile.rs` et `src/lib.rs`.
- Clippy avec les allowances explicites de ces quatre catégories passe (`clippy-baseline-allowances.log`); cela ne remplace pas un Clippy strict vert.
- `cargo fmt --all -- --check`, `git diff --check` et `python3 scripts/verify_cargo_license_state.py` passent. Cargo reste `license=null`, `license_file=null`, `publish=[]`, `no_license_selected`, `redistribution_rights=not_assessed`.

Les commandes et codes figurent dans `commands.txt`; les logs et versions sont dans ce dossier. Le manifeste local vérifie les artefacts du run, puis les manifestes produit et parent sont complétés de manière append-only avec les préfixes préservés.

## Limites

La fixture HTTP est loopback et synthétique; aucune instance Forgejo vivante, CI, modèle/AgentOS, credential réel, publication, service externe, production ou dépense nouvelle n’est impliqué. Le snapshot reste consultatif : un statut Forgejo `success` est non fiable sans vérificateur séparé. La lecture PR/statuts est séquentielle, pas atomique, et ce changement n’autorise ni merge ni écriture de statut. La suite complète `--all-targets` n’a pas été relancée pour cette petite tranche; les tests touchés et la frontière du processus réel ont été exécutés explicitement. Le projet global reste incomplet.
