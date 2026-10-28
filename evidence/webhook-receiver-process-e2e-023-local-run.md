# WEBHOOK-023 — vrai binaire de réception, reprise et réconciliation

Date : 2026-10-22. Worktree : `local/WEBHOOK-005-canonical-links`. HEAD examiné : `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. SHA-256 du nouveau contrat : `c75790d5d2d94908f27002294fba858ab8004cac2c87db9afe73c4cca722f7de`.

## Parcours observé

`tests/webhook_receiver_process_e2e.rs` lance le binaire Cargo réel `forgejo-webhook-local` comme sous-processus, sur une adresse loopback éphémère, avec un secret synthétique en fichier mode `0600` et un journal neuf dans un répertoire mode `0700`. Le corps signé est une issue fictive `closed`; l’état courant fourni ensuite au trait `ForgejoCurrentState` fixture est `open`.

Le binaire répond `401 invalid_signature` au même corps avec une signature altérée. La livraison correctement signée est ensuite acceptée `202` à la séquence 1, ce qui confirme que le refus n’a pas ajouté de ligne; le même UUID et corps renvoyés donnent `202 duplicate=true`. Après arrêt SIGTERM réussi, le test rouvre et vérifie la chaîne du journal, retrouve un seul reçu et le réconcilie vers `open` au moyen d’une lecture fixture. Après redémarrage du même binaire avec le même journal, la relecture du même UUID est encore `duplicate=true` et le journal garde un seul reçu. Un garde RAII termine le sous-processus si une assertion échoue.

La réconciliation ne contacte pas Forgejo : le snapshot `open` et l’URL canonique sont fournis par une fixture locale. Le test exerce le vrai exécutable HTTP et la persistance/reprise, pas la configuration d’un webhook Forgejo, l’API authentifiée, une PR, une vérification indépendante, un merge ou un modèle LLM.

## Commandes et résultats

Depuis la racine du worktree :

```sh
CARGO_TARGET_DIR=/tmp/webhook-receiver-process-e2e-target cargo test --offline --locked --test webhook_receiver_process_e2e -- --nocapture
CARGO_TARGET_DIR=/tmp/webhook-receiver-process-e2e-target cargo test --offline --locked
cargo fmt --all -- --check
git diff --check
CARGO_TARGET_DIR=/tmp/webhook-receiver-process-e2e-target cargo clippy --offline --locked --all-targets -- -D warnings -A clippy::too_many_arguments -A clippy::bool_assert_comparison -A clippy::manual_div_ceil
```

Le contrat subprocess ciblé passe 1/1. La suite finale passe 200 tests, 0 échec, 2 ignorés (24 résumés Cargo). `rustfmt` et `git diff --check` passent. Les journaux finaux sont `webhook-receiver-process-e2e-006-final-full-tests.log`, `webhook-receiver-process-e2e-006-final-rustfmt.log`, `webhook-receiver-process-e2e-006-final-diff-check.log` et `webhook-receiver-process-e2e-006-final-clippy-with-baseline-allowances.log`; le contrat ciblé est consigné dans `webhook-receiver-process-e2e-002-focused.log`.

Le Clippy strict initial sans exceptions (`cargo clippy --offline --locked --all-targets -- -D warnings`) échoue sur des avertissements préexistants hors du nouveau test : trop d’arguments dans `src/agent_runtime.rs:210` et `src/mission_admission.rs:138,236,315`, comparaison booléenne dans `src/lib.rs:1144`, et `manual_div_ceil` dans `src/bin/forgejo-mission-local.rs:594`. Le Clippy final passe en neutralisant seulement ces trois catégories de lint préexistantes; les deux journaux d’échec strict sont conservés sous `webhook-receiver-process-e2e-001-clippy.log` et `webhook-receiver-process-e2e-002-clippy-target.log`. Cela ne signifie pas que le Clippy strict du dépôt passe.

## Limites et prochaine tranche

Aucune source de production n’a été modifiée dans WEBHOOK-023; le changement est un test d’intégration de l’exécutable avec fixtures. Il n’établit aucun gain général de qualité ou de coût et n’est pas un parcours de livraison complet.

L’instance Forgejo locale relevée séparément écoute sur `127.0.0.1:3132`; l’adaptateur actuel demeure sans authentification sur `127.0.0.1:3130`. L’API anonyme ne peut pas lire le dépôt privé et aucun write API n’a été envoyé. Avant un parcours avec état réel, il faut définir une source d’identifiants hôte seule, limiter précisément le compte et ne pas exposer le credential à l’exécuteur, à ses arguments ou à son environnement. Jusqu’à cette décision d’interface et de déploiement, les fixtures sont la frontière correcte. Les readbacks Forgejo sans secret sont dans [WEBHOOK-022](webhook-reconcile-pipeline-003-local-run.md) et [webhook-reconcile-pipeline-005-forgejo-readback.json](webhook-reconcile-pipeline-005-forgejo-readback.json).
