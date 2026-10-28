# FORGEJO-COMMIT-STATUS-PROCESS-E2E-001 — 26 octobre 2026

## Résultat

Le test d’adaptateur précédent validait les pages via des appels de méthode, sans lancer le programme utilisé par l’hôte. `actual_cli_reads_multiple_status_pages_and_refuses_changed_total` lance maintenant le vrai binaire `forgejo-reconcile-local` construit pour l’intégration. Un serveur HTTP fixture lié exclusivement à `127.0.0.1:3130` fournit les réponses à la place de Forgejo; aucun autre hôte ou service n’est visé. Le test est marqué `#[ignore]` parce que le CLI a une origine fixe et que le scénario réserve ce port.

Premier processus : page 1 avec 50 statuts, page 2 avec un statut, `X-Total-Count: 51` stable. Le CLI termine avec succès et produit le snapshot JSON complet : 51 éléments, deux pages, états de page success/failure, état combiné nul, `sequential_non_atomic`, `independent_verification=false` et `merge_authorizing=false`. Second processus : la première page annonce 51, la seconde 52. Le CLI échoue, n’émet aucun JSON sur stdout et donne le message de relecture. Le fixture confirme les quatre routes GET attendues et l’absence d’en-tête Authorization.

## Vérification

Commande explicite (test ignoré par défaut, exécuté seul et sériel) :

```sh
cargo test --locked --offline --test forgejo_reconcile_capabilities_e2e actual_cli_reads_multiple_status_pages_and_refuses_changed_total -- --ignored --exact --test-threads=1
```

Résultat ciblé : 1 passé, 0 échec, 0 ignoré, 3 filtres; sortie dans `local-run.log`. La suite du fichier passe 3/3, avec le nouveau test ignoré comme prévu (`generic-tests.log`). Clippy strict du test avec les allowances historiques explicites, `cargo fmt --all -- --check` et `git diff --check` passent. Le readback Cargo reste `license=null`, `license_file=null`, `publish=[]`; `verify_cargo_license_state.py` retourne `no_license_selected`, `redistribution_rights=not_assessed`, `NOASSERTION`. Le manifeste local vérifie 14 artefacts et les manifestes append-only produit/parent vérifient les empreintes courantes des 22 chemins requis; leurs préfixes sont préservés. Les versions, métadonnées, garde et hashes sont dans ce dossier.

## Portée et limites

C’est un test du processus CLI, de son appel curl et du protocole HTTP local. Il vérifie la forme des routes, la propagation du résultat et le refus de total divergent au niveau processus. Le fixture ne reproduit pas le code serveur Forgejo, ses middlewares, ses permissions ou son index de statuts; aucun serveur Forgejo n’était disponible sur cette machine et aucun n’a été démarré. Le test ne prouve donc pas une intégration live ni la fidélité d’une réponse `success`. Les statuts restent du contexte consultatif, sans liaison à une PR courante ni autorité de fusion.

Le test échoue sans requête si le bind fixe `127.0.0.1:3130` est occupé. Il est ignoré dans les suites génériques et doit être lancé explicitement avec `--test-threads=1`. L’état produit reste sur le HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`, branche `local/WEBHOOK-005-canonical-links`, worktree préexistant très modifié. Aucun modèle/AgentOS, coût nouveau, donnée privée externe, production, publication ou code Forgejo copié.
