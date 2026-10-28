# FORGEJO-PR-BOUND-STATUS-SNAPSHOT-001 — contexte lié à la tête PR relue

Date : 26 octobre 2026. Workspace : `.runtime/webhook-005-workspace`. Commit de base observé : `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. L’arbre était déjà fortement modifié; aucune modification existante n’a été supprimée et aucun commit n’a été créé.

## Changement

Le CLI `forgejo-reconcile-local --pull-request-status-snapshot <numéro> [--host-credential-fd <fd>]` lit la PR du dépôt loopback configuré, obtient les statuts du SHA de tête, puis relit la PR. La liaison compare le numéro, l’état, `merged`, la branche et le SHA de base et le SHA de tête. La réponse `PullRequestSnapshotV1` vérifie le dépôt de base et l’URL canonique de la PR. Si la liaison a changé, le JSON sort en `inconclusive`, `current_pr_head_checked=false`, fournit une action de relecture et le processus sort avec le code 1. Le contenu des statuts de l’ancienne tête est conservé dans `status_snapshot` afin d’expliquer le refus.

La découverte des capacités expose le schéma `forgejo-pull-request-status-snapshot-v1`, l’ordre des lectures, les champs comparés et le périmètre d’identité. L’opération reste lecture seule et consultative; elle n’exécute pas les critères, ne vérifie pas l’origine d’un statut Forgejo et n’autorise pas de merge.

## Vérifications exécutées

Toolchain observée : Rust `rustc 1.95.0 (59807616e 2026-04-14)`, Cargo `1.95.0 (f2d3ce0bd 2026-03-21)`, Bubblewrap `0.9.0`, hôte `aarch64-unknown-linux-gnu`. Artefacts/versions détaillés dans `tool-versions.txt`.

- `cargo test --locked --offline --bin forgejo-reconcile-local pull_request_status_snapshot_ -- --test-threads=1` : 2 réussis, 0 échec, 134 filtrés.
- `cargo test --locked --offline --test forgejo_reconcile_capabilities_e2e actual_cli_discloses_versioned_local_test_capabilities -- --exact --test-threads=1` : 1 réussi.
- Binaire de test `forgejo_reconcile_capabilities_e2e` lancé directement sous `bwrap --unshare-net --unshare-pid --new-session` avec `actual_cli_binds_status_snapshot_to_pr_head_and_refuses_head_drift --exact --ignored --nocapture --test-threads=1` : 1 réussi. Le vrai CLI envoie les trois GET attendus; tête identique donne un snapshot complet, dérive de tête donne un refus structuré `inconclusive` et code 1. Fixture HTTP synthétique sur loopback seulement.
- `cargo test --locked --offline --test forgejo_reconcile_capabilities_e2e -- --test-threads=1` : 3 réussis, 0 échec, 2 ignorés par défaut (les deux réservent le port 3130).
- `cargo clippy --locked --offline --bin forgejo-reconcile-local -- -D warnings` : échec sur 12 alertes de catégories situées hors de la surface de ce lot (`too_many_arguments`, `collapsible_if`, `explicit_auto_deref`, `if_same_then_else`) dans `src/agent_runtime.rs`, `src/forgejo.rs`, `src/mission_admission.rs`, `src/webhook/reconcile.rs`, `src/lib.rs`. Le log strict est conservé, sans suppression de ces alertes ni changement de ces modules dans ce lot.
- Même Clippy avec `-A clippy::too_many_arguments -A clippy::collapsible_if -A clippy::explicit_auto_deref -A clippy::if_same_then_else` : passe. Il s’agit d’un contrôle ciblé avec allowances explicites, non d’un lint strict sans réserve.
- `cargo fmt --all -- --check`, `git diff --check`, et `python3 scripts/verify_cargo_license_state.py` : passent. Readback : `license=null`, `license_file=null`, `publish=[]`, décision `no_license_selected`, droits `not_assessed`.

Les logs sont dans ce dossier : `unit-tests-final.log`, `capabilities-test-final.log`, `pr-bound-process-final.log`, `capabilities-e2e-final.log`, `clippy.log`, `clippy-known-baseline-allowances.log`, `rustfmt.log`, `diff-check.log`, `cargo-license-state.json`. Le test de processus PR reste ignoré dans Cargo générique à cause du port fixe; son invocation isolée réelle est conservée.

## Limites et état de confiance

La suite est séquentielle : les deux lectures de PR autour de la lecture de statuts ne sont pas atomiques. Un changement suivi d’un retour à la même valeur entre ces observations peut ne pas être détecté. `current_pr_head_checked=true` signifie seulement que les deux observations de frontière sont égales; un `success` rapporté par Forgejo n’est toujours pas une preuve indépendante. La sous-réponse du snapshot de commit conserve ses propres limites. Aucun statut n’est consommé par l’autorité de fusion.

Aucune instance Forgejo vivante ou authentifiée n’a été utilisée. Aucun modèle, AgentOS, appel externe, secret, coût, publication ou production n’est impliqué. Cette tranche ne démontre ni le parcours LLM, ni une comparaison contrôlée avec agent+CI conventionnels, ni une sûreté multi-writer. Le projet reste incomplet.
