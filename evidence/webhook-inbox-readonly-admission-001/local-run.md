# WEBHOOK-INBOX-READONLY-ADMISSION-001

Date : 2026-10-24. Dépôt produit local, HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`, worktree modifié. Toolchain : `rustc 1.95.0 (59807616e 2026-04-14)`, `cargo 1.95.0 (f2d3ce0bd 2026-03-21)`.

## Changement

`read_journaled_webhook_event` appelle maintenant `EventJournal::replay_read_only`. L’admission refuse donc une inbox contenant une ligne finale incomplète sans réparer ni tronquer le journal pendant une décision. L’enveloppe, la chaîne de hash et les limites existantes restent vérifiées. Le test ajoute d’abord une livraison signée synthétique, ajoute des octets incomplets à la ligne suivante, puis vérifie `JournalError::Corrupt { line: 2 }` et l’égalité exacte des octets avant/après.

## Preuve rouge / verte

- `red.log` : en remplaçant temporairement le replay non-mutant par `replay_webhook_inbox`, le nouveau test échoue à l’assertion de refus. Le script de reproduction a restauré et comparé le fichier source avant de terminer.
- `contract.log` : `cargo test --offline --locked --test webhook_mission_admission_contract` passe 12/12.
- Le premier run complet `all-targets.log` a découvert une fixture préexistante inadéquate dans `webhook_inbox_capacity`: `File::create` produisait un fichier non privé, refusé par la précondition de sécurité du replay avant l’assertion de capacité. `tests/webhook_inbox_capacity.rs` crée maintenant ce sparse file en mode 0600 dans un dossier 0700. Ce changement est limité au test.
- `capacity.log` : `cargo test --offline --locked --test webhook_inbox_capacity` passe 6/6.
- `all-targets-final.log` : `cargo test --offline --locked --all-targets` passe 435 tests, 0 échec, 27 ignorés, sur 34 groupes de test.

## Contrôles

`rustfmt.log` (`cargo fmt --all -- --check`), `diff-check.log` (`git diff --check`), `cargo-license-guard.log` et `cargo-license-unit-tests.log` passent. Les cinq tests du garde licence réussissent. Le readback garde `license=null`, `license_file=null`, `publish=[]`, `license_decision=unselected` et `redistribution_rights=not_assessed`.

## Portée et limites

La livraison et le journal du test sont synthétiques et temporaires. Ce résultat vérifie le lookup décisionnel local; il ne prouve pas un nouveau parcours Forgejo E2E, une origine webhook authentifiée au-delà du processus d’ingestion, un parcours LLM, ni un gain face à Forgejo + agent + CI conventionnels. Il ne change pas la politique d’écriture du receiver. Les 27 tests ignorés par `--all-targets`, dont plusieurs nécessitent le runner Bubblewrap dédié, ne sont pas comptés comme passés. Aucun code Forgejo n’a été copié, aucune instance de production n’a été touchée et aucun appel payant ou externe n’a été effectué.

## Fichiers de preuve

`red.log`, `contract.log`, `capacity.log`, `all-targets.log` (échec initial de fixture), `all-targets-final.log`, `rustfmt.log`, `diff-check.log`, `cargo-license-guard.log`, `cargo-license-unit-tests.log`, `versions.txt` et `source-sha256.txt` sont couverts par le manifeste local. Le manifeste global couvre également le manifeste local lui-même.
