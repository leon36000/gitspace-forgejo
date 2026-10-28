# WEBHOOK-BRANCH-SNAPSHOT-MISMATCH-PROCESS-001 — refus composé d’une branche mal identifiée

Date : 25 octobre 2026. Exécution locale sur le worktree fortement modifié, basé sur HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. Ce lot ajoute un scénario de processus et son entrée au runner isolé; aucun code Forgejo, service de production, modèle ou secret réel n’a été touché.

## Parcours et verdict

`tests/webhook_receiver_process_e2e.rs::actual_reconcile_cli_process_refuses_mismatched_branch_snapshot_without_journal_mutation` envoie une livraison push signée avec le secret synthétique du test au vrai processus `forgejo-webhook-local`. Après le `202` et l’arrêt du receiver, le vrai `forgejo-reconcile-local` lit le journal, effectue un unique GET branch vers la fixture HTTP locale, et reçoit un JSON HTTP 200 dont `name` désigne une autre branche.

Le résultat est une erreur structurée `inconclusive/resource_mismatch`, `retryable=false`, avec la branche demandée dans les détails et sans `canonical_state`. Le corps synthétique de réponse n’apparaît pas dans la sortie. L’URL API observée vise la branche demandée, l’en-tête `Authorization` est absent et le journal est identique octet par octet avant/après réconciliation. Le scénario ciblé passe 1/1 sous Bubblewrap avec namespaces réseau/PID et compilation offline locked; 60 autres tests du binaire sont filtrés.

Le premier run du nouveau test n’a pas mis en cause le code produit : l’assertion de l’action de remédiation attendait une sous-chaîne trop exacte. Le JSON observé contenait bien `resource_mismatch` non réessayable et le texte `verify that the API resource identity...`; l’assertion a été assouplie à ce fragment stable, sans modifier le comportement de l’adaptateur. Le run final ciblé est vert.

## Cargo sans licence choisie

Le garde courant passe, ainsi que ses 7 tests. Les empreintes des trois sources (`Cargo.toml`, `scripts/verify_cargo_license_state.py` et `scripts/test_cargo_license_state.py`) correspondent à la preuve actuelle [CARGO-LICENSE-NO-SELECTION-007](../cargo-license-no-selection-007/local-run.md). `cargo metadata` reste à `license=null`, `license_file=null`, `publish=[]`; la décision projet est `no_license_selected` et les droits restent `not_assessed`. Aucun SPDX ou droit de redistribution n’a été ajouté.

## Portée et limites

La fixture HTTP est un listener local sur `127.0.0.1:3130`, pas Forgejo. Le test prouve le comportement composé d’un seul refus de snapshot de branche mal identifié, pas la justesse de toutes les réponses API, la concurrence Forgejo, un parcours agent réel, ni un gain par rapport à la CI conventionnelle. Le commit de base identifie le checkout, pas un commit contenant ces changements non enregistrés.

Commandes, sorties résumées, versions et empreintes sont conservées dans les fichiers adjacents et vérifiées par `SHA256SUMS`.
