# WEBHOOK-CREDENTIAL-PREFLIGHT-001 — refuser avant lecture du credential hôte

## Question et changement

`forgejo-reconcile-local` consommait son FD de credential hôte dès qu’un FD optionnel était fourni, avant de valider la ressource des événements sélectionnés. Une livraison HMAC signée mais mal formée, ou visant un dépôt hors du périmètre configuré, pouvait donc échouer sur le FD avant de produire le refus structuré attendu. Le chemin sans refus amont risquait aussi de consommer un credential à usage unique inutilement.

Le CLI calcule maintenant `needs_host_credential` à partir des événements sélectionnés. Il lit le FD uniquement si au moins un `event_resource_ref` est valide et cible exactement l’owner et le dépôt configurés. Un lot ne contenant que des événements mal formés ou hors périmètre suit la réconciliation normale, qui renvoie un verdict `inconclusive`/refus structuré sans lecture amont. Un lot mixte lit le credential une fois pour traiter ses événements dans le périmètre; chaque ressource hors périmètre reste soumise au refus de l’adaptateur. Le descripteur n’est jamais transmis à `curl`.

## Preuve rouge puis correction

Deux tests focalisés ont échoué avant la correction. Ils signent des corps synthétiques avec HMAC, écrivent les livraisons dans le journal local, puis fournissent `i32::MAX` comme FD invalide :

- `malformed_selected_delivery_is_refused_before_host_credential_read` recevait l’erreur de niveau invocation `cannot secure the host credential descriptor` au lieu du résultat structuré `invalid_repository`.
- `out_of_scope_selected_delivery_is_refused_before_host_credential_read` recevait la même erreur de FD avant le refus structuré `upstream_forbidden`.

Les journaux rouges correspondants sont `red-test.log` et `out-of-scope-red-test.log`. Après la correction, `cargo test --offline --locked --bin forgejo-reconcile-local host_credential_read -- --nocapture` passe 4/4; les deux nouveaux cas rendent `inconclusive`, zéro tentative de lecture amont et le code de refus attendu. `cargo test --offline --locked --bin forgejo-reconcile-local tests::signed_journal_deliveries_reconcile_through_the_local_http_adapter -- --exact` passe également 1/1 pour contrôler le cas en périmètre : le credential synthétique est lu et la fixture loopback reçoit la requête attendue.

La suite déjà lancée sur l’arbre courant, `cargo test --offline --locked --all-targets`, se termine avec 431 réussis, 0 échec, 23 ignorés. Le résumé exact et les sorties sont conservés dans `all-targets.log`. `cargo fmt --all -- --check`, `git diff --check` et la garde de métadonnées Cargo passent; le garde conserve `license=null`, `license_file=null`, `publish=[]`, décision `unselected` et droits de redistribution `not_assessed`.

## Portée et limites

Tests unitaires/processus du CLI local et fixture HTTP loopback uniquement. Aucun appel à une instance Forgejo, aucune issue/PR ou fusion réelle de ce lot, aucun agent LLM, AgentOS, RAG ou mémoire, aucun secret/credential réel, aucun service externe et aucune mesure comparative n’ont été utilisés. Le cas de credential en périmètre emploie uniquement une donnée synthétique. Le chemin de lot mixte est codé mais n’a pas un nouveau test spécifique dans cette tranche. Le test `i32::MAX` prouve l’ordre de refus face à un FD invalide; il ne modélise pas un fournisseur système de credentials ni la sécurité d’un processus exécuté sous un autre UID. La suite Cargo comporte 23 tests ignorés, qui ne sont pas présentés comme vérifiés.

## Reproduction et références

Commandes vertes :

```text
cargo test --offline --locked --bin forgejo-reconcile-local host_credential_read -- --nocapture
cargo test --offline --locked --bin forgejo-reconcile-local tests::signed_journal_deliveries_reconcile_through_the_local_http_adapter -- --exact
cargo test --offline --locked --all-targets
cargo fmt --all -- --check
git diff --check
python3 scripts/verify_cargo_license_state.py
```

Les deux logs rouges, les logs verts, les versions d’outils et le hash SHA-256 de la source sont dans ce répertoire. Le HEAD de base est consigné dans `tool-versions.txt`; les sources restent des changements non commités du worktree et ne doivent pas être attribuées à un commit de livraison.
