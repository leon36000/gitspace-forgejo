# WEBHOOK-RECONCILE-BATCH-MERGE-BINDING-001

Date : 2026-10-25. Extension locale de la preuve `WEBHOOK-RECONCILE-MERGED-COMMIT-BINDING-001`, dans le worktree basé sur HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. L’arbre préexistant reste modifié; ce lot ajoute un contrat processus et son allowlist, sans modifier le comportement produit.

## Contrat couvert

Le chemin batch transporte le `merge_commit_sha` canonique d’une pull request déjà fusionnée jusqu’au JSON de sortie. Le test existant `actual_reconcile_cli_process_keeps_issue_and_pull_request_number_collision_separate` vérifie maintenant explicitement le SHA dans `results[1].canonical_state`.

Le nouveau scénario `actual_reconcile_cli_process_keeps_missing_merge_sha_inconclusive_in_mixed_batch` fait passer deux livraisons signées dans le vrai récepteur local, lit ensuite deux snapshots HTTP synthétiques dans le vrai CLI, et reçoit une issue valide plus une PR marquée fusionnée sans `merge_commit_sha`. Le CLI retourne le code 1 et un JSON structuré : la réponse globale est `inconclusive`; l’issue garde `current_state_read`; la PR reste `inconclusive`, nomme `merge_commit_sha` dans l’erreur et n’a aucun `canonical_state`. Le journal reste identique aux octets acceptés. Les deux tests vérifient l’absence de l’en-tête Authorization.

Le runner `scripts/run_webhook_reconcile_cli_e2e.py` les lance séparément sous Bubblewrap avec `--unshare-net`; les deux terminent 1/1. Il s’agit de fixtures HTTP locales sur loopback, pas d’une instance Forgejo. Aucun code Forgejo n’est copié ou modifié.

## Vérifications

- `cargo test --offline --locked --all-targets -- --test-threads=1` : 34 groupes, 480 réussis, 0 échec, 44 ignorés; exit `0`.
- `cargo build --release --locked --offline`, `cargo fmt --all -- --check` et `git diff --check` passent.
- Les sorties brutes et codes de retour se trouvent dans ce dossier.

Le manifeste local couvre 22 artefacts et passe 22/22; les commandes, sorties, versions et empreintes de sources sont reproductibles depuis ce dossier.

## Licences, portée et limites

Le hash courant de `Cargo.toml` correspond au hash du rapport `CARGO-LICENSE-STATE-EXPLICIT-001`; son manifeste local passe 8/8. Vérification fraîche : `scripts/verify_cargo_license_state.py` retourne `status=verified`, puis `scripts/test_cargo_license_state.py` passe 7/7. L’état est `license=null`, `license_file=null`, `publish=[]`, décision `no_license_selected`, droits de redistribution `not_assessed`. Ce lot ne choisit ni n’attribue de licence.

Ces tests renforcent le chemin de sérialisation/refus batch, sans démontrer l’instance réelle, l’intégration LLM, AgentOS, un vérificateur indépendant ou une validation comportementale après merge. Ils ne mesurent pas de gain face à une forge et une CI conventionnelles. Les empreintes des sources et versions exactes figurent dans les fichiers de ce dossier.
