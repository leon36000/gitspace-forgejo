# CONTINUATION-404 — publication Forgejo Release, readback canonique et correction des versions

Date : 28 octobre 2026. Worktree : `.runtime/webhook-005-workspace`; branche `local/WEBHOOK-005-canonical-links`; HEAD de base `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`.

## Changement

Le réconciliateur traite maintenant le webhook natif `release` avec les actions `published`, `updated` et `deleted`. Il extrait un ID positif limité au dépôt, relit la ressource via `GET /api/v1/repos/{owner}/{repository}/releases/{release_id}` et n’expose que ID, tag, draft et prerelease. Il omet les notes, assets et URL non fiables; construit l’URL de page depuis l’origine configurée et le tag courant; signale `updated_at: null`, absent de la réponse Forgejo v17. Une réponse 404 demeure inconclusive. La route exacte GET est ajoutée à la policy en lecture seule et couverte contre les variantes non canoniques, sous-routes, mauvais dépôts, query/fragment et méthode mutante.

Le contrat webhook évolue à v9 tout en conservant v8. Les schémas de résultat sont single 19, batch 23 et capabilities 28 (`forgejo-reconcile-capabilities-v28`). La migration des fixtures a trouvé plusieurs attentes restées en 18/22/27; elles ont été alignées dans les tests Rust et scripts Python concernés. Aucun Cargo.toml/Cargo.lock, code Forgejo/GitSpace, dépendance ou licence n’a été ajouté/modifié par le support release.

## Source upstream examinée

Forgejo officiel v17.0.0, commit `0d791421f85ca13a63b665053d81ea765fbbc571`. Les trois fichiers ciblés, leurs empreintes, symboles et limites d’examen sont consignés dans [forgejo-source-review.txt](forgejo-source-review.txt). Audit ciblé seulement; aucun test Go upstream. Aucun code upstream copié.

## Vérifications locales

- `cargo test --offline --locked --test webhook_mission_admission_contract -- --test-threads=1`: 15 réussis, 0 échec.
- `cargo test --offline --locked --test webhook_reconcile_contract release_webhook_reads_current_release_and_projects_only_bounded_identity_fields -- --exact --nocapture`: 1 réussi.
- `cargo test --offline --locked --bin forgejo-reconcile-local capabilities_list_exact_merge_target_profiles -- --nocapture`: 1 réussi après synchronisation de l’assertion 27→28.
- `cargo test --offline --locked --test webhook_quarantine_cli -- --test-threads=1`: 7 réussis après synchronisation single/batch 18/22→19/23.
- `cargo test --offline --locked --all-targets -- --test-threads=1`: code 0; 757 réussis, 0 échec, 189 ignorés, 38 groupes. Voir [journal complet](continuation-404-all-targets-default-target.log). Les tests ignorés ne sont pas couverts par ce run.
- `cargo fmt --all -- --check` et `git diff --check`: passent.
- `ast.parse` des trois scripts Python modifiés: passe; ces parcours Python/live n’ont pas été lancés.

Le test unitaire de projection release figure aussi parmi les 100 tests réussis de `webhook_reconcile_contract`; `reconciliation_policy_allows_only_exact_release_read_route` figure parmi les 165 tests réussis du binaire reconcile.

## Portée particulière du test de processus release

Avant cette reprise, le scénario `actual_reconcile_cli_process_reads_current_release_by_id_and_omits_notes` avait été exécuté et rapporté 1/1 dans la sortie d’outil de la reprise précédente : GET exact, sans Authorization, champs non fiables omis et journal inchangé. Le journal brut n’avait pas été persisté. La tentative de le rejouer ici a échoué avant la copie des sources vers le test avec `ENOSPC` (seulement 768 Ko libres à ce moment). Le traceback est conservé dans [release-process-e2e.log](release-process-e2e.log). Ce dernier échec ne contredit pas l’observation précédente; il ne constitue pas un nouveau test produit. Le résultat antérieur reste une observation non durable, pas une preuve reproductible enregistrée.

Deux premières suites globales ont aussi exposé des assertions de capabilities et de single/batch périmées. Une suite sur CARGO_TARGET_DIR tmpfs a ensuite révélé que le test Bubblewrap n’exposait pas un binaire stocké hors de son namespace; le contrôle final a donc utilisé le target directory du workspace et a passé. Les journaux intermédiaires sont conservés pour le diagnostic.

## État des documents et espace disque

`docs/architecture.md` avait été vidé après un échec d’écriture de patch. La version exacte référencée avant l’incident par le manifeste avait SHA-256 `50f9cc08c658cc0e43cbdf3d996c2478bd080025c5f1512afaae9e113b547d2a`; aucun contenu identique n’a été retrouvé. Le document a été reconstruit depuis l’instantané local du 23 octobre, SHA-256 `b6ac8b678f15f96fb314a4a47942c612dceaf09af5c45be57bb35f1e4ae1434f`, puis annoté comme dérivé. Ce n’est pas une restauration exacte. Une copie exacte fournie ultérieurement pourra remplacer ce dérivé après comparaison.

La partition système était pleine pendant la tentative processus; les tests globaux ont ensuite libéré de l’espace temporaire et la suite complète a pu s’exécuter. En fin de lot, il restait environ 379 Mo sur `/`; aucun fichier n’a été supprimé. Le runner processus release n’a pas été rejoué.

## GitLab PC4 et limites

Aucun accès distant, email, credential, routeur, service GitLab ou production n’a été touché dans ce lot. Les derniers reçus disponibles (CONTINUATION-342/378/399) indiquent que PC4 ne répond pas au HTTPS depuis PC6 et que le credential ciblé par courriel ne vise pas `root/gitspace-forgejo`; les credentials administrateur ou exposés restent inutilisés. Aucune référence distante, fetch, commit de transfert ou push n’a eu lieu. Cette reprise n’a pas retenté le réseau, puisqu’aucun changement de connectivité n’a été signalé.

Aucune intégration AgentOS/Forge Code/RAG/mémoire ni exécution LLM n’est démontrée par ce lot. Aucun gain comparatif. Projet incomplet.

Empreintes SHA-256 des fichiers source/test concernés au moment du reçu : [source-sha256.txt](source-sha256.txt). `Cargo.toml`/`Cargo.lock` apparaissent déjà modifiés dans le worktree sale par des lots antérieurs; ce lot release n’y ajoute aucune dépendance et n’a pas modifié leurs octets.
