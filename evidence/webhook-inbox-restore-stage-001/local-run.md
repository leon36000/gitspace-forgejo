# WEBHOOK-INBOX-RESTORE-STAGE-001 — staging local d’un snapshot validé

## Résultat

Ajout du mode CLI `forgejo-webhook-local --stage-restore-inbox <snapshot.jsonl> <nouvel-inbox.jsonl> <sha256-attendu>`. Il valide la forme du hash fourni séparément, impose les permissions privées du fichier source et de son répertoire, valide le journal sous verrou (séquences, chaîne d’intégrité et clés d’idempotence), compare le SHA-256 attendu, écrit exclusivement un nouveau fichier puis relit et revalide les octets et le hash. L’écriture synchronise le fichier et son répertoire. Le reçu JSON ne contient pas les charges utiles du journal.

Le mode ne remplace ni ne sélectionne une inbox active. Le reçu déclare `quiescent_restore_point=false`, `post_snapshot_delivery_replay=operator_required` et `signature_authenticity=not_established`. Un SHA-256 fourni par l’opérateur vérifie l’égalité à cette valeur, pas l’origine du snapshot. La reprise de la séquence et la déduplication du journal continuent après staging.

## Vérifications

- Le test processus ciblé passe **13/13**, dont huit cas ajoutés : staging suivi d’un append et d’une déduplication; refus pour hash mal formé ou divergent, journal corrompu même si le hash fourni correspond, source absente ou liée, destination existante ou symlinkée. Les cinq cas d’export déjà présents passent également.
- `cargo fmt --all -- --check`, `git diff --check`, le garde Cargo sur la licence et l’audit des liens documentaires passent.
- `cargo clippy --locked --offline --bin forgejo-webhook-local -- -D warnings` échoue avant l’analyse du binaire sur 22 diagnostics de la bibliothèque, localisés dans `src/agent_runtime.rs`, `src/forgejo.rs`, `src/mission_admission.rs`, `src/webhook/reconcile.rs` et `src/webhook.rs`. Aucun diagnostic ne vise `src/bin/forgejo-webhook-local.rs`; le build processus et les tests du binaire ont réussi. Ce contrôle Clippy n’est donc pas vert.
- Le test rouge de développement avait été observé avant le changement : le binaire initial traitait `--stage-restore-inbox` comme une adresse d’écoute et sortait avec `invalid listen address`. La sortie brute n’a pas été conservée dans ce dossier; cette observation antérieure n’est pas comptée comme un artefact reproductible de ce run.

## Couverture et limites

Le test appelle le binaire local et vérifie les fichiers privés et les reçus, mais ne simule pas une panne électrique au milieu de l’écriture, ne quiesce pas un receiver actif et ne rejoue pas les événements acquittés après la copie. Le test n’établit pas l’authenticité de la sauvegarde. Le répertoire de preuve capture un test local hors ligne; aucun Forgejo vivant, agent LLM/AgentOS ou système GitLab n’a été appelé pour cette vérification. La licence du produit reste non choisie et les droits de redistribution non évalués.

## Références

- Implémentation CLI : `src/bin/forgejo-webhook-local.rs:120-177`, `:273-330`.
- Tests processus : `tests/webhook_inbox_snapshot.rs:249-471`.
- Décision d’architecture et limites de restauration : `docs/architecture.md`.
- État et suite : `docs/checkpoint.md`.
- Sorties brutes : `snapshot-restore-tests.log`, `clippy.log`.
