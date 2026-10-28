# CONTINUATION-044 — lier les métadonnées de dépôt de l’issue courante

## Défaut constaté

La réconciliation vérifiait déjà le numéro de l’issue et comparait `html_url` à l’URL attendue, mais elle ignorait l’objet `repository` retourné avec l’issue. Le test ajouté a d’abord échoué sur un instantané avec le bon numéro et la bonne URL, mais `repository.full_name = other-owner/other-repository`; le code rendait pourtant un état canonique positif.

## Correction

Après la vérification du numéro de ressource, `src/webhook/reconcile.rs::validate_current_issue_repository` lie au dépôt attendu les champs Forgejo présents `full_name`, `name` et `owner`. Leur forme est vérifiée : `owner` est une chaîne dans `RepositoryMeta`. Toute valeur étrangère renvoie `ResourceMismatch`; une valeur mal typée ou un objet repository sans champ d’identité renvoie `InvalidCurrentStateField`. L’absence du champ ou `null` restent acceptés pour compatibilité avec les réponses partielles; ces cas ne donnent pas un second signal d’identité, mais les contrôles existants du numéro et de l’URL canonique restent requis.

## Source amont

Forgejo v17.0.0, commit exact `0d791421f85ca13a63b665053d81ea765fbbc571`. `modules/structs/issue.go::Issue.Repo` porte `json:"repository"`; `RepositoryMeta` définit `Name`, `Owner string` et `FullName`. Le blob a été lu par `git show HEAD:modules/structs/issue.go` depuis `/tmp/forgejo-v17-base-cas-poc`; le dépôt scratch est globalement fortement modifié, mais ce fichier n’a aucun diff par rapport au commit et son SHA-256 est `e6cb7890f1d7045b2423465d1e452b6f91c4d71c746957aefc9bade286d10006`. Aucun code Forgejo n’a été copié.

## Vérification

- TDD rouge : le cas de `full_name` étranger a d’abord réussi à tort; après correction, les cas valides, absents/null, noms/owners étrangers, objet vide et owner de mauvaise forme sont couverts.
- `tests/webhook_reconcile_contract.rs`: 40/40.
- `tests/webhook_reconcile_pipeline.rs`: 3/3.
- Le processus E2E `actual_reconcile_cli_process_refuses_issue_repository_metadata_mismatch` traverse le vrai receiver, l’inbox durable et le vrai CLI. Une fixture HTTP loopback fournit la bonne URL mais un `RepositoryMeta` étranger; le CLI produit `inconclusive/resource_mismatch`, n’émet aucun état canonique et laisse l’inbox byte-for-byte inchangée. Exécution dédiée par `scripts/run_webhook_reconcile_cli_e2e.py`, sous Bubblewrap avec namespaces réseau et PID séparés : 1/1.
- `cargo test --locked --offline --all-targets -- --test-threads=1`: 570 réussis, 0 échec, 81 ignorés, 34 groupes. Les tests ignorés, y compris ceux qui réservent le port fixe, restent ignorés dans ce run; le scénario ajouté a été lancé à part.
- `cargo fmt --all -- --check`, `git diff --check` et garde Cargo passent. Les métadonnées restent `license=null`, `license_file=null`, `publish=[]`, `no_license_selected`, `NOASSERTION`, `redistribution_rights=not_assessed`.

Les commandes, logs bruts, version de l’outil, provenance et hashes des sources sont archivés dans ce dossier.

## Portée et limites

Le test de processus utilise un vrai receiver et un vrai CLI Rust, mais l’API Forgejo est une fixture locale synthétique. Il n’établit pas qu’un serveur Forgejo live peuple toujours ces champs, et ne teste ni Go upstream, ni LLM/AgentOS, ni comparaison de productivité. L’environnement courant ne fournit pas `go`; le scratch CAS/LFS reste non exécutable sans le toolchain. Aucun serveur, donnée privée, service externe, production, licence, dépense ou publication n’a été engagé.
