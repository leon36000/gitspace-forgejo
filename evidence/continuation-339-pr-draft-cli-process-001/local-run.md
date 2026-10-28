# CONTINUATION-339 — preuve processus de la projection PR `draft`

Date : 28 octobre 2026. Travail local dans le worktree sur HEAD de base `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`, branche `local/WEBHOOK-005-canonical-links`.

## But et changement

CONT-338 avait prouvé la projection du champ à l’adaptateur et au contrat, mais le test de processus correspondant est ignoré dans les suites Cargo génériques. Le scénario existant `actual_reconcile_cli_process_reads_pull_request_current_state_from_private_http_fixture` est maintenant explicite sur la fraîcheur : l’événement synthétique `pull_request/synchronize` journalise `draft=true`; le GET API loopback retourne `draft=false`; le vrai processus `forgejo-reconcile-local` émet `canonical_state.draft=false`. La route HTTP observée est `/api/v1/repos/forge-admin/agent-forge-core/pulls/23`; aucun en-tête Authorization n’est envoyé.

Le premier essai isolé a atteint la vérification de l’état canonique, puis a échoué sur des assertions historiques qui attendaient une notification d’avis `inline review comment`. Le payload de ce scénario est un événement `pull_request/synchronize` sans `review`. Selon `review_notification_from_event`, seul un événement d’avis/commentaire produit cette valeur; le scénario ne doit pas inventer une notification. Les quatre assertions incorrectes ont été remplacées par une assertion `review_notification == null`. Le run isolé suivant passe.

## Résultats reproductibles

Commande du processus (journal complet avec invocation Bubblewrap) :

```text
python3 scripts/run_webhook_reconcile_cli_e2e.py --scenario actual_reconcile_cli_process_reads_pull_request_current_state_from_private_http_fixture
```

Résultat : 1 test passé, 0 échec; 167 scénarios du même binaire filtrés. La sandbox emploie Bubblewrap 0.9.0, `--unshare-net` et les seuls sockets loopback nécessaires à receiver/fixture. Le code source est monté en lecture seule dans la sandbox. La réponse est servie par une fixture Rust à `127.0.0.1:3130`, pas par Forgejo.

Vérifications complémentaires après modification : `cargo fmt --all -- --check` réussit; `git diff --check` réussit; `python3 -m py_compile scripts/run_webhook_reconcile_cli_e2e.py` réussit; `python3 scripts/verify_cargo_license_state.py` retourne `metadata_policy_verified`. Aucun changement de dépendance ou manifeste Cargo n’a été effectué dans ce lot. Le rapport, logs ciblé et versions se trouvent dans ce dossier.

La suite complète `cargo test --offline --locked --all-targets -- --test-threads=1` n’a pas été répétée ici : le run CONT-338 demeure la dernière preuve agrégée (728 réussis, 0 échec, 158 ignorés). Le scénario de processus de cette tranche a, lui, été explicitement exécuté par le runner isolé.

## Portée et limites

Prouvé : le journal receiver local, le vrai binaire de réconciliation, le transport HTTP vers la fixture allowlistée et la sélection de `draft` depuis l’API courante malgré le webhook périmé. `draft` demeure descriptif avec `merge_authorizing=false`; ce run ne prouve ni l’exactitude de Forgejo live ni une autorité de fusion.

Non prouvé : instance Forgejo/GitLab, accès aux courriels/credentials, AgentOS, LLM ou travail d’agent, PR ou merge, vérification post-fusion, comparaison contrôlée et gain produit. Aucun réseau externe, secret, écriture Git distante, commit, publication ou production n’a été utilisé.

Le checkout reste lourdement modifié : HEAD inchangé, 19 entrées suivies modifiées et 1 726 entrées non suivies à l’instantané avant l’ajout des fichiers de reçu; aucune modification antérieure n’a été écrasée. Tous les fichiers de ce reçu sont liés par les manifestes append-only locaux et du projet.
