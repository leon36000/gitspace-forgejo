# CONTINUATION-340 — refus processus des champs PR `draft` absents ou mal typés

Date : 28 octobre 2026. Base locale : `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`, branche `local/WEBHOOK-005-canonical-links`.

## Changement et comportement exercé

Le contrat de bibliothèque CONT-338 refusait déjà une réponse pull-request API sans `draft` booléen. Après la preuve positive au processus CONT-339, le chemin fail-closed n’était encore éprouvé que par cette bibliothèque. Le nouveau test ignoré par défaut `actual_reconcile_cli_process_refuses_missing_or_malformed_current_pull_request_draft` envoie deux webhooks signés `pull_request/synchronize` vers le receiver Rust et garde `draft=false` dans chacun. Le serveur HTTP loopback sert ensuite deux réponses pour la même PR : la première omet `draft`, la seconde l’encode comme chaîne `"false"`.

Pour chaque invocation du vrai `forgejo-reconcile-local`, le test exige un code de sortie non-nul et une réponse JSON structurée : `verdict=inconclusive`, `error.code=invalid_current_state`, `details.field=draft`, `retryable=false`, avec l’action d’examiner/corriger la réponse source avant reprise. Aucun `canonical_state` n’est émis; ainsi le champ webhook `false` ne devient pas une valeur de repli. Les deux appels sont sur la route `/api/v1/repos/forge-admin/agent-forge-core/pulls/26`, sans Authorization. Le journal signé reste byte-identical après les deux refus.

Le test est ajouté à l’allowlist `scripts/run_webhook_reconcile_cli_e2e.py` et exécuté explicitement sous Bubblewrap `--unshare-net`; la fixture réservée `127.0.0.1:3130` est le seul endpoint utilisé par cette sonde. Résultat final après formatage : 1 test passé, 0 échec, 168 tests du target filtrés; ce test contient deux processus CLI et deux réponses malformées.

## Vérification

- `cargo test --offline --locked --test webhook_reconcile_contract -- --test-threads=1`: 87/87.
- Runner Bubblewrap ciblé: 1/1; log complet : [isolated-process.log](isolated-process.log).
- `scripts/test_cargo_license_state.py`: 12/12.
- `scripts/verify_cargo_license_state.py`: `metadata_policy_verified`; `license=null`, `license_file=null`, `publish=[]`, `license_decision=no_license_selected`, `redistribution_rights=not_assessed`, `spdx_package_license_declared=NOASSERTION`.
- `cargo fmt --all -- --check` et `git diff --check`: code 0 après exécution de `rustfmt --edition 2024 tests/webhook_receiver_process_e2e.rs`.

Cargo.toml et Cargo.lock n’ont pas été changés; aucune dépendance ni licence n’a été ajoutée. Le marqueur `NOASSERTION` reste un avis non-assertif dans le metadata local, pas l’attribution d’une licence produit.

## Limites

Ce run prouve le comportement de deux réponses API simulées au vrai processus client, pas Forgejo ou GitLab vivant. Il ne démontre pas la création de PR, la fusion, l’isolation générale d’un agent, AgentOS/LLM, l’exécution post-fusion ou un gain comparatif. Les formes null et nombre sont encore exercées au niveau bibliothèque, pas par ce nouveau scénario processus. Le test dédié est `#[ignore]` dans Cargo et dépend du runner isolé.

Le worktree demeure fortement modifié et non commité; HEAD et branche n’ont pas changé. Aucun courriel, credential, service externe, écriture distante ou production n’a été utilisé.
