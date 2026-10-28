# FORGEJO-NATIVE-BRANCH-CREATE-001

## Parcours observé

Le runner a démarré une instance Forgejo jetable, privée et limitée à loopback, avec un dépôt privé et un lecteur API non-admin (`pull=true`, `push=false`). Il a créé une branche par Git HTTP. L'événement natif `create` a été reçu et lié à la tâche Forgejo correspondante, livrée avec HTTP 202. Le corps reçu contient le nom court `agent/native-delete-proof`, `ref_type=branch` et le SHA initial `b887eb208b916f8a97703e0b6e5a403face7c47c`.

Après cet événement, le runner a poussé un commit supplémentaire (`1777c73687b9743f87003cb208b8c09730036dfd`), supprimé la branche puis confirmé l'absence de la ref distante. Forgejo a émis `delete`, également reçu en HTTP 202. Le runner a recréé la branche à la tête avancée avant de réconcilier les deux livraisons. Pour `create` comme pour `delete`, le vrai CLI a effectué une lecture API avec l'identité read-only et rendu `current_state_read` avec la tête courante `1777c73687b9743f87003cb208b8c09730036dfd`. Ainsi le SHA annoncé par `create` est effectivement périmé et n'est pas traité comme l'état canonique. Le journal reste identique avant/après les lectures.

Forgejo a confirmé la configuration et la désactivation du hook. L'instance et le receiver sont sortis avec code 0; aucun listener attendu ne restait. Aucun service distant n'a été contacté.

## Source, outils et provenance

- Source Forgejo officielle examinée: tag `v17.0.0`, commit `0d791421f85ca13a63b665053d81ea765fbbc571`; références de fichiers et blobs ciblés dans [upstream-source-reference.txt](upstream-source-reference.txt). Le scratch est modifié ailleurs par des POC; seuls les quatre fichiers listés ont été vérifiés sans diff au commit.
- Binaire local: `forgejo version 17.0.0+gitea-1.22.0`, SHA-256 `e5ec437cd6357e8d1b4580cb47ab1daa58a32b33971a63db63446a535d33adb7`; le commit de build est inconnu.
- Rust/Cargo `1.95.0`, Bubblewrap `0.9.0`; le receiver et le reconciler ont été compilés `--offline --locked` dans Bubblewrap avec réseau et PID namespace isolés. La provenance des sources, outils et binaires est dans [result.json](result.json); les deliveries, tâches, readbacks et deux résultats sont conservés comme JSON.
- SHA initial `b887eb208b916f8a97703e0b6e5a403face7c47c`; SHA API courant après recréation `1777c73687b9743f87003cb208b8c09730036dfd`.

## Vérifications

- `cargo test --offline --locked --all-targets -- --test-threads=1`: 525 réussis, 0 échec, 64 ignorés, 34 groupes. Les tests ignorés, doctests et scénarios processus dédiés ne sont pas inclus dans ce total; le parcours Forgejo natif décrit ci-dessus a été lancé séparément.
- `tests/webhook_handler_contract.rs`: 19/19; `tests/webhook_reconcile_contract.rs`: 34/34.
- Tests Python `tests.test_forgejo_emitted_webhook_e2e`: 63/63; `py_compile`, `cargo fmt --all -- --check`, `git diff --check` et garde de métadonnées Cargo passent.
- Le garde relit `license=null`, `license_file=null`, `publish=[]`, `decision=no_license_selected`, `redistribution_rights=not_assessed`. Aucune licence n'a été choisie.

Les premiers `--all-targets` ont mis au jour deux assertions qui considéraient encore `create` comme événement inconnu; elles ont été remplacées par des exemples réellement inconnus (`repository` et `star`). Une assertion du test d'annulation de superviseur exigeait que le cgroup systemd soit vide immédiatement après la sortie; sous charge elle voyait brièvement un PID, alors que l'essai isolé passait. Le test attend désormais jusqu'à deux secondes, avant tout nettoyage de garde, et la suite finale observe le cgroup vide. Le code du superviseur n'a pas été changé dans cette tranche.

## Limites

Un seul dépôt, une branche et une instance locale démontrent le wire et la réconciliation courante; ils ne prouvent pas une garantie universelle de livraison, l'isolation multi-tenant, un parcours LLM/AgentOS, une PR/fusion, une comparaison agent+CI ou un gain de productivité. Aucun code Forgejo n'est adopté. La licence du crate reste non choisie et les droits de redistribution non évalués. Le projet reste incomplet.
