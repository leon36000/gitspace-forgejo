# CONTINUATION-360 — refus d’une nouvelle livraison après changement de milestone seul

Date : 28 octobre 2026. Worktree local : branche `local/WEBHOOK-005-canonical-links`, HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. Le dépôt était déjà fortement modifié; aucun reset, staging, commit, suppression, push ou accès à GitLab n’a eu lieu. Aucun secret, service externe ou modèle n’a été utilisé.

## Lacune vérifiée

CONT-358 transporte le milestone courant lu par API dans le `untrusted_work_item`, et son empreinte est déjà intégrée au contexte. Le test processus existant prouvait que la première admission transmettait milestone ID 12 malgré ID 99 dans le webhook; il ne prouvait pas qu’un changement de milestone après le premier claim empêchait de réutiliser la même génération. Le nouveau scénario conserve le titre, le corps, les labels, les exigences protégées et la base; seul le milestone API passe de ID 12 à ID 13. Un second webhook porte encore l’ancienne valeur ID 12.

Résultat du vrai receiver, du CLI Rust et du host MCP sous le runner Bubblewrap : la première admission reste la source du handoff. La seconde admission répond `verdict=inconclusive`, `error=execution_idempotency_conflict` avec une action demandant une nouvelle génération approuvée par l’hôte. Les octets du fichier de handoff de la première admission restent identiques. Le refus est dû à la nouvelle représentation courante de la requête; les champs du webhook ne sont pas la source canonique. La logique produit déjà présente a suffi : ce lot ajoute une preuve processus ciblée, sans changement de code de production.

## Vérifications

- `python3 scripts/run_forgejo_mission_delivery_process_e2e.py --only actual_cli_current_issue_labels_reach_mcp_context_from_signed_webhook` : code 0, scénario 1/1 réussi dans Bubblewrap avec namespaces réseau et PID isolés; source montée en lecture seule, endpoints Forgejo en fixture locale. Journal : [context-e2e.log](context-e2e.log). L’essai d’invocation directe du script a d’abord reçu `Permission denied` car il n’a pas le bit exécutable; l’invocation Python correcte a réussi.
- `cargo test --offline --locked --all-targets -- --test-threads=1` : code 0, 38 groupes, 739 réussis, 0 échec, 166 ignorés. Journal : [all-targets.log](all-targets.log). Le nouveau scénario `#[ignore]` n’est pas inclus dans ce total; il est exécuté séparément ci-dessus.
- `cargo fmt --all -- --check` et `git diff --check` : code 0, journaux [rustfmt.log](rustfmt.log) et [diff-check.log](diff-check.log). L’E2E compile aussi le fichier de test et l’exécute.
- `cargo metadata --offline --locked --no-deps --format-version 1` et `scripts/verify_cargo_license_state.py` : code 0. Readback exact : `license=null`, `license_file=null`, `publish=[]`, `decision=no_license_selected`, droits `not_assessed`, marqueur d’export `NOASSERTION`. Les 12 tests de la garde passent avec `python3 scripts/test_cargo_license_state.py`.
- Une première commande `python3 -m unittest tests.test_cargo_license_state` a échoué au chargement du module, car ce test est dans `scripts/`; l’erreur est conservée dans [license-tests-import-error.log](license-tests-import-error.log). L’invocation correcte par chemin de fichier passe 12/12 : [license-tests.log](license-tests.log). Aucun échec de la garde n’est masqué.

Les sorties Cargo directes et la garde sont dans [cargo-metadata.json](cargo-metadata.json) et [license-guard.json](license-guard.json). La toolchain annoncée par le runner : rustc 1.95.0, cargo 1.95.0, Bubblewrap 0.9.0, Linux aarch64. Aucun téléchargement réseau n’a été demandé; les tests utilisent les caches Cargo copiés dans une sandbox hors ligne.

## Portée et limites

Ce test établit une seule séquence locale (`n=1`) et ne prouve pas un lock distribué ni l’absence de mutation immédiatement après le second readback. Le serveur Forgejo est un fixture HTTP, pas une instance live. Le host MCP constate toujours la première admission; le nouveau test s’arrête avant toute exécution de modèle, PR, vérification indépendante ou fusion. Aucun AgentOS/LLM, coût ou benchmark n’est démontré. Le résultat de la garde de licence confirme l’état Cargo technique, pas des droits de redistribution ni une décision juridique. Aucun composant Forgejo/GitSpace n’est copié et aucune dépendance n’est ajoutée.

## Fichiers liés

La seule modification de code de ce lot est le test de processus `tests/forgejo_mission_delivery_process_e2e.rs`; l’implémentation préexistante qui hash le milestone est confirmée par ce parcours. Les documents canoniques mis à jour sont le checkpoint, la matrice des refus et le plan d’implémentation. SHA-256 des sources : [source-hashes.txt](source-hashes.txt). Les manifestes locaux et les index append-only lient ce reçu et les sorties.
