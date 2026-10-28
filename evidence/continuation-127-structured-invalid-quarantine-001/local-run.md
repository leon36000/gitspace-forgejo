# CONTINUATION-127 — refus JSON actionnable pour une disposition invalide

Date : 26 octobre 2026. Worktree `/home/pc6/Documents/Codex/2026-10-21-mission-construire-une-forge-con-ue/.runtime/webhook-005-workspace`, branche `local/WEBHOOK-005-canonical-links`, HEAD de base `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. Dépôt fortement modifié avant et après le lot; aucun commit n’est créé.

## Changement

`src/bin/forgejo-reconcile-local.rs::run_reconciliation` convertit maintenant le rejet de `quarantine_for_source` en enveloppe JSON `inconclusive`, code `invalid_quarantine_disposition`, retryable false, plutôt qu’en texte stderr non structuré. Le helper garde la forme de réponse existante : schéma single 5 avec `delivery_id` pour une sélection, schéma batch 6 avec `requested_delivery_count` pour plusieurs. `details` déclare `validation_scope=all_quarantine_dispositions_in_journal`, `canonical_state_created=false`, `host_credential_read=false` et `upstream_calls_attempted=false`; l’action demande de préserver le journal et d’inspecter les dispositions hors ligne avec un outil de confiance. Le CLI garde son code de sortie 1. Il n’écho pas les champs ou hashes du marqueur rejeté et n’essaie ni de le sauter, ni de le réparer.

## Preuve test-first et vérification

Le test-processus `real_cli_fails_closed_on_malformed_or_rebound_quarantine_dispositions` dans `tests/webhook_quarantine_cli.rs` lance le vrai exécutable sur deux journaux à chaîne complète validée. Chaque inbox contient une livraison invalide (source d’avis PR sans action), une disposition adverse, puis une ou deux livraisons `issues/opened` actives valides. Le premier cas a `reason_field` numérique et sélectionne une livraison active; le second lie le marqueur à un hash source remplacé et sélectionne deux livraisons, couvrant les enveloppes single et batch. Chaque invocation reçoit un credential synthétique via FD pipe. Le test vérifie code 1, stderr vide, JSON versionné exact, code/action/détails, zéro tentative de lecture rapportée, pipe toujours plein, et comparaison octet-pour-octet du journal avant/après. Le journal est rejoué avant l’appel CLI pour valider les quatre lignes de chaque cas.

Le test a d’abord échoué comme prévu : le binaire envoyait le diagnostic texte vers stderr et ne produisait pas de réponse JSON. Les logs rouges sont gardés dans `red-test.log`. Une erreur de compilation du helper de fixture (`String` déplacé) et une première assertion de forme single erronée sont aussi conservées comme problèmes de test corrigés, pas comme échecs du produit. Le test processus final passe 1/1; les assertions finales relient single à schema 5 et batch à schema 6.

La suite `cargo test --offline --locked --all-targets` passe 621 tests, 0 échec, 106 ignorés, 36 groupes. Les tests ignorés ne sont pas exécutés par cette commande. Après ajout des assertions finales de version, le test processus ciblé a été rejoué et passe 1/1. `cargo fmt --all -- --check` et `git diff --check` finaux passent. Les sorties sont dans `all-targets.log`, `green-focused.log`, `rustfmt-final.log` et `diff-check.log`.

## Métadonnées Cargo

Lecture fraîche via `cargo metadata --offline --locked --no-deps --format-version 1` : paquet `agent-forge-core`, `license=null`, `license_file=null`, `publish=[]`, `decision=no_license_selected`, `redistribution_rights=not_assessed`, marqueur d’export personnalisé `NOASSERTION`. `python3 scripts/verify_cargo_license_state.py` passe et `python3 -m unittest -v scripts.test_cargo_license_state` passe 9/9. Cela confirme l’absence de choix Cargo et n’évalue pas les droits GitSpace/Forgejo/dépendances. Aucun fichier Cargo n’est modifié.

## Limites

Aucun service Forgejo, modèle, AgentOS, accès externe ou coût n’est engagé ici. Le test CLI ne déclenche pas de requête réseau : le code retourne avant lecture du FD et la fixture vérifie que le pipe reste intact. Les events et marqueurs sont synthétiques; ce n’est ni un HMAC réauthentifié ni un test d’instance Forgejo. L’inbox complète utilisée dans ce test reste inchangée; ce lot ne prouve pas cette propriété si une queue finale est aussi tronquée, car le `replay()` existant peut réparer une queue incomplète avant la validation des dispositions. Le hash-chain confirme la structure/intégrité enregistrée, pas la provenance ou la correction du marqueur. Pas de résultat de productivité ni de gain comparatif.

Toolchain et hash des sources/docs : `toolchain.txt`, `source-revisions.txt`, `source-hashes.txt`, `cargo-metadata.json`, `cargo-license-summary.json`. Le prochain travail doit sélectionner un risque restant non déjà couvert; la mission globale demeure incomplète.
