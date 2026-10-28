# CONTINUATION-341 — refuser une PR brouillon au préflight CLI réel

Date : 28 octobre 2026. Worktree local, HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`, branche `local/WEBHOOK-005-canonical-links`.

## Lacune fermée

`validate_merge_preflight_state` refusait déjà une PR brouillon au niveau bibliothèque (`tests/forgejo_merge_preflight.rs::draft_pull_request_is_refused`), mais aucun test du vrai processus `forgejo-reconcile-local --preflight` n’exerçait cette décision. Le nouveau test ignoré par défaut `actual_preflight_cli_refuses_draft_pull_request_without_writes` fournit au CLI un reçu de preuve synthétique correctement signé pour les SHA candidat/base attendus. La fixture API loopback renvoie une PR ouverte, mergeable, liée au dépôt/base corrects, mais `draft=true`.

Résultat processus : refus structuré `draft_pull_request`, `verdict=inconclusive`, `merge_authorized=false`, `merge_attempted=false`; le code de sortie n’indique pas une réussite. La fixture ne voit qu’un GET de PR, exige le chemin attendu et refuse toute requête supplémentaire; aucun Authorization n’est envoyé. Cela vérifie le refus avant toute écriture, pas un merge. Tests : préflight processus ciblé 1/1; contrat unitaire `forgejo_merge_preflight` 13/13.

## Runner réparé pour atteindre le test

Le premier essai a échoué avant Cargo : le runner local n’appelait plus `compile_targets` avec la signature du helper partagé. Après ajout des cibles binaires et du cache offline, la compilation a réussi mais l’export des cinq binaires dépassait la limite de 128 MiB. Le test n’exécute que le binaire `forgejo-reconcile-local`; le runner ne compile/exporte donc que celui-ci. Le lancement a ensuite mis en évidence l’argument obligatoire `test_name` absent de l’appel `sandbox.run`; il a été ajouté. Les trois échecs du runner sont conservés dans `runner-signature-mismatch.log`, `runner-artifact-budget-failure.log` et `runner-missing-test-name.log`. Le dernier run a compilé puis exécuté le test 1/1, journal [isolated-process.log](isolated-process.log).

La correction du runner est restreinte au scénario et aux cibles nécessaires; les bornes d’artefacts du helper sont conservées. Bubblewrap applique `--unshare-net --unshare-pid`, monte la source en lecture seule et ne rend accessible que l’API fixture loopback.

## Autres vérifications

- `cargo test --offline --locked --test forgejo_merge_preflight -- --test-threads=1`: 13 passés, 0 échoué.
- `cargo fmt --all -- --check`: code 0.
- `git diff --check`: code 0.
- `python3 -m py_compile scripts/run_forgejo_preflight_cli_e2e.py`: code 0.
- Le statut Cargo de licence reste `license=null`, `license_file=null`, `publish=[]`, `no_license_selected`, `not_assessed`, `NOASSERTION`; la garde complète de 12 tests est consignée dans CONT-340. Aucun manifeste Cargo, aucune dépendance et aucune licence n’ont été ajoutés dans cette tranche.

## Limites

La preuve est une fixture locale et une preuve signée synthétique; elle ne vérifie pas une PR ou une règle de protection sur Forgejo vivant, ni un secret ou une identité d’approbateur réel. Le préflight est lecture seule et ne teste pas la chaîne de merge. Pas d’AgentOS/LLM, de comportement post-fusion ou de benchmark. Les autres scénarios du runner préflight ne sont pas relancés dans ce lot. Aucun courriel, service externe, Git distant, commit, publication ou production n’a été touché.
