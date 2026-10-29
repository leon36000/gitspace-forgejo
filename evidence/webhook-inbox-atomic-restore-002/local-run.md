# WEBHOOK-INBOX-ATOMIC-RESTORE-002

Date locale : 28 octobre 2026. Worktree `local/WEBHOOK-005-canonical-links`, base observée `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`, déjà fortement modifié avant ce lot. Les changements de ce lot restent locaux.

## Lacune et changement

Le writer commun à `--snapshot-inbox` et `--stage-restore-inbox` ouvrait directement le nom final avec `create_new`, puis écrivait et synchronisait les données. Une fin de processus entre l’ouverture et la fin de l’écriture pouvait laisser un fichier final partiel. La validation du journal à la lecture suivante l’aurait refusé, mais le chemin ressemblait à une destination publiée.

Le writer ouvre maintenant le répertoire parent privé avec `O_DIRECTORY | O_NOFOLLOW`, vérifie son propriétaire et son mode `0700`, crée un inode anonyme Linux `O_TMPFILE` en mode privé, écrit tous les octets et appelle `sync_all`. `linkat` via `/proc/self/fd/<fd>` publie ensuite cet inode au nom demandé; la création du lien échoue si ce nom existe déjà. La vérification après publication exige un fichier régulier, le propriétaire courant, le mode `0600` et un seul lien; le répertoire est synchronisé. Les erreurs d’ouverture `EOPNOTSUPP`/`EINVAL` indiquent le besoin d’un système de fichiers compatible; les autres erreurs restent des refus sans publication.

Un point d’arrêt de test est compilé uniquement sur Linux avec `test-crash-injection`, et un garde existant interdit cette feature dans les builds release. Trois scénarios processus couvrent : arrêt après sync de l’inode anonyme avant `linkat`; arrêt après `linkat` avant le `fsync` du répertoire; création concurrente du nom final pendant le point d’arrêt pré-publication. Un wrapper RAII tue et récolte l’enfant après toute sortie anticipée du test.

Le run complet a aussi trouvé une assertion obsolète dans `gate_request_paths_cannot_mix_repositories_from_two_profiles`: elle interdisait `GET /repos/forge-admin/mission-012-clamp`, route de visibilité désormais explicitement permise pour CONTINUATION-438. Le cas a été retiré de la liste des chemins étrangers; la route est couverte par `reconciliation_policy_allows_only_exact_configured_repository_visibility_read`.

## Résultats

- Tests rouges TDD : chaque nouveau point d’arrêt a d’abord échoué parce que le CLI n’exposait pas ce point d’arrêt; le test de collision a échoué parce que la barrière ne savait pas encore reprendre. Ces échecs valident le harnais et la nouvelle surface de test; ils ne sont pas présentés comme une reproduction directe du fichier partiel de l’ancien writer.
- `cargo test --locked --offline --features test-crash-injection --test webhook_inbox_snapshot -- --test-threads=1` : **16 réussis, 0 échec**; sortie finale dans [`restore-feature-tests-final.log`](restore-feature-tests-final.log).
- `cargo test --locked --offline --all-targets -- --test-threads=1` : **784 réussis, 0 échec, 204 ignorés**, 39 groupes; sortie finale dans [`all-targets-final.log`](all-targets-final.log).
- `cargo fmt --all -- --check` et `git diff --check` : passent.
- `python3 scripts/verify_cargo_license_state.py` : politique inchangée, `license=null`, `license_file=null`, `publish=[]`, `no_license_selected`, droits de redistribution `not_assessed`, SPDX `NOASSERTION`.
- `cargo clippy --locked --offline --all-targets --features test-crash-injection --message-format short` : sortie 0 avec avertissements; aucun avertissement dans `forgejo-webhook-local.rs` ou `webhook_inbox_snapshot.rs`. Cinq avertissements apparaissent dans d’autres zones de `forgejo-reconcile-local.rs`, hors de la ligne de test ajustée. Le passage strict `cargo clippy --locked --offline --bin forgejo-webhook-local --test webhook_inbox_snapshot --message-format short -- -D warnings` échoue sur **22 diagnostics de la bibliothèque**. Détails dans [`clippy.log`](clippy.log) et [`clippy-final-all-targets-feature.log`](clippy-final-all-targets-feature.log). L’arbre de travail étant déjà fortement modifié, cette preuve ne date pas ces diagnostics par rapport à la base Git.

## Portée et limites

Exécuté sur Linux aarch64, Rust/Cargo 1.95.0, avec le système de fichiers local où `O_TMPFILE` et `/proc/self/fd` fonctionnent. Les systèmes non Linux retournent un refus explicite; ils n’ont pas été construits ici. Le test pré-publication démontre l’absence de nom final après `SIGKILL`; après publication, le test lit les octets et métadonnées pendant la pause, tue le processus, puis valide la chaîne complète du journal. La collision concurrente est séquencée à la frontière de publication et le contenu préexistant reste identique. Les tests ne simulent pas une panne électrique, un noyau défaillant, une corruption de disque ou une erreur de `fsync` du répertoire; lors d’un crash après `linkat`, le nom peut déjà exister avec les octets complets sans durabilité confirmée.

Le résultat reste un snapshot/restauration staged. Il n’active pas le journal, n’arrête pas les writers, ne rejoue pas les deliveries acquittées après le snapshot et n’authentifie pas la provenance du snapshot. Aucun LLM/AgentOS, Forgejo live, service distant, courriel, credential ou benchmark n’a été utilisé dans ce lot. Aucun code Forgejo ou GitSpace n’a été copié.

La mission globale reste incomplète : comparaison contrôlée, AgentOS/LLM réutilisable sans dépense, inventaire de licences et droits GitSpace restent ouverts.
