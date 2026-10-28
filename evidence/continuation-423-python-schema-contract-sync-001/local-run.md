# CONTINUATION-423 — synchroniser les contrats de schéma des adaptateurs Python

Date : 28 octobre 2026 (America/Montreal). Worktree : `local/WEBHOOK-005-canonical-links`, HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. Le checkout est déjà fortement modifié et contient des fichiers non suivis; aucun changement antérieur n’a été supprimé, déplacé ou commité.

## Écart observé

Le CLI Rust `src/bin/forgejo-reconcile-local.rs` définit `RECONCILE_SINGLE_SCHEMA_VERSION = 19` et `RECONCILE_BATCH_SCHEMA_VERSION = 23`. Le test Rust `actual_cli_discloses_versioned_local_test_capabilities` publie ces valeurs dans le document de capacités. Plusieurs runners/validateurs Python consommaient encore les versions single `18` et batch `22`. Les fixtures Python de la branche courante utilisaient déjà `19/23`; leurs tests échouaient avant toute validation du contenu canonique, de l’ordre ou du verdict.

Avant la correction, quatre cas représentatifs (commentaire issue, batch branche, batch PR, batch push) échouaient avec les erreurs d’enveloppe correspondant aux anciennes versions. Un nouveau test de contrat a d’abord échoué parce que le module Python n’exposait aucune constante raccordée aux versions Rust. La sortie rouge est dans `red.log`.

## Correction

Le runner partagé `run_forgejo_live_webhook_reconcile_emitted.py` expose maintenant les constantes `RECONCILE_SINGLE_SCHEMA_VERSION = 19` et `RECONCILE_BATCH_SCHEMA_VERSION = 23`. Les validateurs single/batch qui dépendent de ce contrat les réutilisent dans les runners webhook live, branche-delete, push, commentaire PR/reviewers et replay natif. Le test `ReconciliationSchemaContractTests` compare ces constantes aux déclarations du CLI Rust pour éviter une dérive silencieuse.

Aucun contrat Rust, stockage, credential, configuration Forgejo, dépendance ou déclaration de licence n’a changé.

## Vérifications

- Contrat rouge avant correction : 4 cas représentatifs échouent; test de cohérence Python/Rust échoue parce que la constante Python manque.
- Après correction : contrat Python/Rust et quatre cas représentatifs passent **5/5**.
- `tests/test_forgejo_emitted_webhook_e2e.py` : **172 réussis, 0 échec**. Cela résout les 16 échecs et 20 erreurs précédemment observés dans ce module; le total augmente de 171 à 172 après ajout du test de cohérence.
- `tests/test_forgejo_webhook_native_replay_e2e.py` : **30 réussis, 0 échec**.
- `tests/test_forgejo_live_org_team_reviewer_readback.py` : **20 réussis, 0 échec**.
- `cargo test --offline --locked --test forgejo_reconcile_capabilities_e2e actual_cli_discloses_versioned_local_test_capabilities -- --exact --test-threads=1` : **1 réussi**, confirmant les versions de capacités du binaire Rust.
- Garde de licence Cargo : **12/12**. `cargo metadata --locked --offline --no-deps --format-version 1` donne `license=null`, `license_file=null`, `publish=[]`, décision `no_license_selected`, droits `not_assessed` et marqueur `NOASSERTION`.
- AST Python des cinq runners touchés et du test : passe. `git diff --check` passe. Recherche ciblée : aucun comparateur runner `schema_version != 18/22` ne demeure.

Les sorties et versions sont conservées dans les fichiers adjacents; `commands.txt` donne les commandes. Les tests Python sont des validateurs à fixtures, et le test Rust vérifie le document local de capacités.

## Limites et suite

`forgejo` et `go` ne sont pas disponibles dans PATH. Aucun serveur Forgejo, webhook natif, création de release, pull request ou fusion n’a été exécuté dans ce lot. Ce résultat prouve la compatibilité des validateurs avec les versions déclarées par le CLI local, pas un parcours live de bout en bout. Aucun modèle/AgentOS n’a été invoqué; aucun coût ni accès nouveau.

Le garde Cargo reste configuré sans licence choisie et sans publication. Aucune décision de droits de redistribution n’est prise.

Suite utile : câbler le nouveau contrat Release dans l’orchestration du runner plutôt que de le laisser au niveau parseur/binding, puis le vérifier avec une instance Forgejo de test locale lorsqu’elle est réellement disponible. Jusqu’à cette preuve, la prise en charge Release est de la plomberie testée, pas une intégration live. Les parcours de livraison complets, la comparaison contrôlée et les autres critères de la mission restent ouverts.
