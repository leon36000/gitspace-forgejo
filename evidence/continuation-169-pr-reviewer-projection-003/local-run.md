# CONTINUATION-169 — projection canonique des reviewers demandés

Date locale : 26 octobre 2026. Worktree produit : `/home/pc6/Documents/Codex/2026-10-21-mission-construire-une-forge-con-ue/.runtime/webhook-005-workspace`, HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`; ce HEAD est la base du worktree, pas l'identité d'un commit contenant les modifications testées. L'arbre était déjà fortement modifié. Les empreintes des sources testées et les commandes de contrôle sont consignées avec ce reçu.

## Lacune et changement

La réconciliation d'une PR projetait son état, sa tête, sa base et son merge, mais omettait les reviewers auxquels une revue est demandée. Le champ est maintenant projeté depuis la réponse canonique Forgejo dans les états de PR et de commentaire de conversation PR. `null` ou champ absent reste « inconnu » (`None`), tandis qu'une liste vide signifie connu-vide. Les identités ne contiennent que `id` et `login`; le parseur rejette l'objet malformé, les ID non positifs/dupliqués, les logins vides/avec contrôles et les listes de plus de 1024 items; le résultat est trié par ID. Les schémas CLI single/batch passent respectivement à 6/7 et la découverte de capacité documente le nouveau champ.

Les données Forgejo restent la source canonique : le payload `review_requested` ou `review_request_removed` déclenche un readback, mais ne détermine pas la liste. Le parcours a ajouté le reviewer `forge-reader` (ID 2), relu `[{"id":2,"login":"forge-reader"}]`, puis l'a retiré et relu `[]`. La réconciliation en lot des deux événements s'est faite après retrait; les deux événements ont donc, correctement, l'état canonique courant `requested_reviewers=[]`. Ce batch a fait deux lectures API au total, une de la PR et une du commentaire de conversation, en réutilisant la lecture de la PR. Les événements webhook sont liés aux livraisons `bcb8b200-49f8-468d-a4f6-45e4e2d0c73f` et `36c70eff-87c3-4408-8f87-d34f9970ca11`; les API Forgejo répondent 201 puis 204.

## Source amont

L'audit local Forgejo est au dépôt scratch `/tmp/forgejo-v17-base-cas-poc`, HEAD officiel v17.0.0 `0d791421f85ca13a63b665053d81ea765fbbc571`. `modules/structs/pull.go::PullRequest` déclare `RequestedReviewers` sous `requested_reviewers` et `RequestedReviewersTeams` sous `requested_reviewers_teams`. `services/convert/pull.go::ToAPIPullRequest` initialise les deux collections, appelle `LoadRequestedReviewers` et `LoadRequestedReviewersTeams`, puis remplit les deux champs. `modules/structs/user.go::User.UserName` est exposé comme `login`. Ces fichiers étaient propres par rapport à HEAD dans le scratch au contrôle; leurs empreintes de blob sont dans `source-hashes.txt`.

Le lot ne projette actuellement que les reviewers utilisateurs. `requested_reviewers_teams` reste omis et doit faire l'objet d'une décision/test séparé si cette dimension est nécessaire. Aucun code Forgejo ou GitSpace n'a été copié.

## Tests et défauts découverts

TDD du sérialiseur Rust : l'assertion de la nouvelle projection a d'abord échoué car le champ canonique n'était pas présent; elle a passé après le changement.

Deux premiers runs live conservés dans `...-001` et `...-002` ont échoué de façon exploitable dans le harnais : le premier comptait une lecture API au lieu des lectures PR + commentaire; le second appliquait au résultat imbriqué d'un batch les hypothèses d'enveloppe du résultat single. Le calcul du nombre de lectures et la validation de l'élément batch ont été ajustés. Le troisième run est passé. Les rapports JSON de ces trois runs conservent erreurs, livraisons, readbacks et nettoyage.

Vérifications ciblées relancées sur l'état courant :

| Commande | Résultat |
|---|---:|
| `cargo test --offline --locked --test webhook_reconcile_contract` | 67 passés |
| `cargo test --offline --locked --bin forgejo-reconcile-local` | 151 passés |
| `cargo test --offline --locked --test forgejo_reconcile_capabilities_e2e` | 3 passés, 2 ignorés (ports fixes) |
| `cargo test --offline --locked --test reconcile_config_contract` | 4 passés |
| `cargo test --offline --locked --test reconcile_journal_lock_timeout` | 3 passés |
| `cargo test --offline --locked --test webhook_quarantine_cli` | 5 passés |
| `python3 -m unittest discover -s tests -p 'test_forgejo_emitted_webhook_e2e.py'` | 123 passés |
| `python3 -m unittest discover -s tests -p 'test_forgejo_webhook_native_replay_e2e.py'` | 27 passés |

`py_compile`, `cargo fmt --all -- --check`, `python3 scripts/verify_cargo_license_state.py` et `git diff --check` ont également réussi. Les sorties sont conservées dans les fichiers `*.log` de ce répertoire. Ce n'est pas une relance de la suite `--all-targets`.

## Parcours Forgejo live local

Commande :

```text
python3 scripts/run_forgejo_live_webhook_reconcile_emitted.py --forgejo-binary /tmp/forgejo-17.0.0 --repo-root . --state-dir /tmp/forgejo-reviewer-projection-169-run003-state --evidence-dir evidence/continuation-169-pr-reviewer-projection-003 --remove-review-request --batch-pull-request-events
```

Verdict du runner : `passed`; Forgejo `17.0.0+gitea-1.22.0`, construit avec Go 1.26.9, binaire SHA-256 `e5ec437cd6357e8d1b4580cb47ab1daa58a32b33971a63db63446a535d33adb7`; le commit de build du binaire n'est pas établi. L'instance jetable et le webhook sont limités à loopback. Le build Rust du runner utilise Bubblewrap avec réseau/PID séparés, environnement nettoyé, sources staged en lecture seule et Cargo offline/locked. Les binaires et Forgejo tournent cependant sous le même UID hôte.

Le receiver a aussi rejoué un ID déjà traité : HTTP 202, `duplicate=true`, 9 lignes et 77 494 octets du journal avant/après, SHA-256 inchangé `eaa234f8d45c0fa485f2518435a09bd439f4c9bff1e619fef3c7ef5a3c10ae34`. La livraison originale est re-sérialisée depuis l'objet journalisé; les octets de la requête initiale ne sont pas conservés. Le readback final confirme arrêt des processus/listeners suivis et désactivation du hook.

Les JSON de sortie, les hashes et la provenance du build sont dans ce dossier; `SHA256SUMS` couvre les artefacts du runner, le rapport, les logs et les pièces de traçabilité.

Les deux runs rouges `continuation-169-pr-reviewer-projection-001` et `-002` conservent chacun leurs six fichiers et leur manifeste local. Les 12 artefacts plus les deux manifestes locaux sont maintenant référencés dans les index globaux par append; le reçu de l'opération et ses vérifications de préfixe sont dans `red-run-index-append.json`.

## Portée et limites

Observation live `n=1`, instance locale jetable, même UID pour Forgejo/receiver/CLI, aucune exécution de code candidat hostile, pas de LLM, AgentOS, RAG ou service mémoire et pas de mesure de productivité/coût. La boucle webhook/readback démontre l'intégration locale de ce champ, pas les permissions de fusion, l'autorité de vérification, la sûreté multi-tenant, l'exhaustivité des autres champs Forgejo, ni le parcours complet de la mission. Le binaire local ne révèle pas son commit de build. La licence produit Cargo n'est pas choisie; les droits de redistribution ne sont pas évalués. Le projet reste incomplet.
