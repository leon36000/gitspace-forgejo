# CONTINUATION-373 — distinguer milestone inconnu et non assigné au niveau processus

Date : 28 octobre 2026. Worktree `local/WEBHOOK-005-canonical-links`, HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. Le checkout reste modifié; tous les changements existants sont préservés. Aucun reset, staging, commit, fetch, push ni suppression. Versions, statut Git condensé et disponibilité des outils : `environment.txt`; sources liées par SHA-256 : `source-sha256.json`.

## Contrat exercé

Le test `actual_reconcile_cli_process_distinguishes_unknown_and_unassigned_pull_request_milestone_in_batch` traverse le vrai receiver, le journal persistant et le vrai binaire `forgejo-reconcile-local` pour deux PR distinctes. Le premier webhook `milestoned` (PR 52) contient un milestone historique ID 99, tandis que l’API courante omet le champ `milestone`. Le second webhook `demilestoned` (PR 53) et l’API courante portent un `null` explicite.

Le receiver acquitte les deux événements aux séquences 1 et 2. Le CLI lit chaque ressource une fois (deux GET, sans Authorization) et retourne `current_state_read` pour le batch. La PR 52 est projetée `milestone.observation=unknown`; la PR 53, `unassigned`. Les deux états donnent `value=null`, mais conservent provenance API courante, fraîcheur `current_api_read` et confiance `untrusted_user_content`. Le journal est byte-identical après lecture; le milestone historique du webhook n’apparaît pas dans stdout.

## Test-first et vérifications

Avant ajout au `TEST_NAMES` du runner, `--scenario` est refusé par argparse (exit 2; `runner-allowlist-red.log`). Après allowlisting, le processus ciblé passe 1/1 sous Bubblewrap `--unshare-net --unshare-pid`. Le contrôle rustfmt initial a demandé un ajustement de quelques lignes; après formatage, le même scénario repasse 1/1 avec 177 tests filtrés (`process-e2e-final.log`). La compilation est offline, `/home` et `/tmp` sont masqués et des limites de ressources sont appliquées par le runner.

```text
python3 scripts/run_webhook_reconcile_cli_e2e.py --scenario actual_reconcile_cli_process_distinguishes_unknown_and_unassigned_pull_request_milestone_in_batch
cargo fmt --all -- --check
python3 scripts/test_cargo_license_state.py
python3 scripts/verify_cargo_license_state.py
git diff --check
```

Rustfmt, py_compile, le contrôle robuste des espaces finaux, `git diff --check` et les 12 tests de garde Cargo passent. `cargo metadata` confirme `license=null`, `license_file=null`, `publish=[]`, `no_license_selected`, `redistribution_rights=not_assessed`, SPDX `NOASSERTION`. Aucune licence n’est choisie, aucun manifest Cargo ni dépendance n’est modifié. Les tests et le validateur unitaires déjà présents n’ont pas été modifiés; le lot ajoute une vérification processus et l’allowlist du runner.

## Portée et limites

Deux PR, deux événements signés synthétiques et deux réponses HTTP loopback; aucun serveur Forgejo vivant ou émission native n’a participé. Aucun AgentOS ou modèle n’a été invoqué; ces fixtures couvrent uniquement la frontière receiver/API/CLI. Cela ne prouve pas la stabilité des ressources après leur GET, un merge contrôlé, un comportement post-fusion ou un gain comparatif. Le test n’exécute pas la suite complète.

La suite logique est l’audit de provenance hôte des exigences protégées, du digest et de la commande de vérification déjà marqué ouvert dans le checkpoint; le champ digest venu d’un harnais ne prouve pas son autorité.
