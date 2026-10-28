# Observations test-first — CONTINUATION-367

Date : 28 octobre 2026. Les commandes ci-dessous ont été réellement exécutées avant la modification de l’implémentation, pendant cette reprise. Le transcript d’outil conserve leur sortie; ce fichier en résume le diagnostic. Les sorties brutes du premier passage n’ont pas été redirigées vers des fichiers à cet instant.

1. `cargo test --offline --locked --test webhook_reconcile_contract current_api_assignees_distinguish_unknown_from_known_empty -- --exact` — exit 101. L’assertion de l’issue avec `assignees: null` comparait `left: Null` à l’objet attendu `{items: [], source: current_forgejo_resource_api, freshness: current_api_read, trust: untrusted_forgejo_identity}`.
2. `cargo test --offline --locked --bin forgejo-reconcile-local capabilities_list_exact_merge_target_profiles` — exit 101. Le runtime produisait `schema_version=25` alors que le test de contrat mis à jour attendait 26. Une première invocation de ce test avec `--exact` n’avait trouvé zéro test; seul le passage sans `--exact` constitue le résultat rouge.
3. `PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=tests python3 -m unittest -q test_forgejo_emitted_webhook_e2e.IssueAssigneeSnapshotTests.test_current_issue_api_null_means_empty_only_when_field_is_present` — exit 1. Échec attendu car `normalize_current_issue_api_assignees` n’existait pas.

Précaution de provenance : les fichiers `red-*.log` et `test-first-exit-codes.txt` voisins ont été produits par un second script lancé après l’implémentation; ils contiennent tous des sorties vertes. Malgré leur nom, ils ne sont pas les sorties rouges listées ci-dessus et ne doivent pas être utilisés comme telles.
