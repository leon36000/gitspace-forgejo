# CONTINUATION-300 — compatibilité des actions `issues` Forgejo v17

Date : 27 octobre 2026. Worktree local fortement modifié au HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`; aucun commit, reset, suppression, publication, déploiement ou accès élargi. Aucun changement à `Cargo.toml`/`Cargo.lock`, aucune dépendance ni dépense ajoutée. Aucun code Forgejo ou GitSpace copié.

## Lacune et preuve amont

Forgejo v17.0.0 au commit `0d791421f85ca13a63b665053d81ea765fbbc571` active quatre catégories distinctes dans `models/webhook/webhook.go::Webhook.EventCheckers`, mais `modules/webhook/type.go::HookEventType.Event` leur donne toutes l’en-tête réseau `issues`. Les chemins notifier produisent six actions supplémentaires pour une issue : `assigned`, `unassigned`, `label_updated`, `label_cleared`, `milestoned`, `demilestoned`. L’ancien contrat local n’admettait que `opened`, `closed`, `reopened`, `edited`; les six notifications étaient rejetées par le receiver et par le reconciler. Détails ligne par ligne et quatre SHA-256 dans `upstream-source.txt` et `docs/source-inventory.md`.

L’audit amont est une lecture statique de quatre fichiers, d’un commit et d’une version; aucun test Go ni serveur Forgejo n’a été exécuté. Les fichiers ciblés sont propres au commit, mais le scratch complet `/tmp/forgejo-v17-base-cas-poc` reste modifié par d’autres POC. Les anciennes notes plus étroites, notamment CONT-284, restent dans l’historique et sont explicitement corrigées par le recontrôle présent.

## Modifications locales

- Le validateur courant accepte les dix actions documentées sous l’en-tête `issues`; `synchronized` reste refusé sous ce header.
- Le contrat machine-readable v5 est figé à l’ancien tableau d’actions. Le nouveau v6 expose les dix actions; les deux gardent `webhook_payload_is_authoritative=false`.
- Les capacités HTTP du receiver passent du schéma supérieur 6 à 7. La découverte du CLI passe de `forgejo-reconcile-capabilities-v18` à `v19`; les schémas single/batch ne changent pas.
- Le test de processus du receiver accepte les dix actions, vérifie leur séquence et leur présence en journal, puis vérifie refus avant append pour action absente, non chaîne et `synchronized` sous `issues`.
- Le test de contrat de réconciliation traite `label_updated`, lit une issue courante par identifiant, et prouve que l’état/titre provient du readback canonique plutôt que des champs périmés du webhook.

## Vérifications

Les quatre assertions ciblées nouvelles/élargies échouaient avant le changement : receiver répondait 400 à `assigned`; reconciler renvoyait `InvalidEventPayloadField("action")`; capacités restaient au schéma 6 et v18. Journaux `*-red.log` gardent ces résultats. Un premier run complet a également trouvé une assertion unitaire qui attendait encore v18; elle a été mise à jour puis le run complet final a passé.

Sept tests ciblés passent (receiver ingestion, handler capabilities, action contract reconcile, canonical readback, CLI capabilities, receiver-process capabilities, et receiver-process action set), un par test. La suite finale `cargo test --locked --offline --all-targets` passe avec 716 réussis, 0 échec, 155 ignorés, sur 38 blocs de résultat (`all-targets-final.log`). Les ignorés ne sont pas exécutés. `cargo check --locked --offline --all-targets`, `cargo fmt --all -- --check`, `git diff --check` et `python3 scripts/verify_cargo_license_state.py` passent. Le readback de garde reste `license=null`, `license_file=null`, `publish=[]`, `no_license_selected`, redistribution `not_assessed`, `NOASSERTION`.

Les index locaux, worktree et projet sont append-only; le générateur a vérifié 25 entrées locales, 14 empreintes source et les préfixes historiques des index. Les dernières versions de chaque entrée locale/source sont liées aux index du worktree et du projet; les anciennes versions d’un fichier modifié restent conservées comme historique append-only.

## Portée et limites

Le processus receiver local et le binaire CLI sont réellement démarrés par les tests; la fixture reconcile utilise un `ForgejoCurrentState` de test, pas le serveur Forgejo. Il n’y a pas de parcours live contre Forgejo, pas d’exécution des Actions/runners, pas d’agent LLM/AgentOS, pas de tokens, coût ou comparaison de valeur. Le test confirme compatibilité de contrat et lecture canonique en fixture, pas identité de l’émetteur, correction des données Forgejo ni gain général.

La mission globale reste incomplète. Restent notamment la comparaison appariée agent/CI conventionnelle, les critères de refus cumulés non encore prouvés, les droits de redistribution et la démonstration d’un chemin LLM/AgentOS autorisé sans nouveau coût.
