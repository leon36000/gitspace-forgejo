# Snapshot consultatif des statuts de commit Forgejo (26 octobre 2026)

## Résultat

Le CLI local expose maintenant `--commit-status-snapshot <sha-complet> [--host-credential-fd <fd>]`. Il n’accepte qu’un SHA complet hexadécimal canonique, lit uniquement le profil loopback fixe `forge-admin/agent-forge-core`, et interroge la première page de `GET /api/v1/repos/forge-admin/agent-forge-core/commits/{sha}/status?limit=50&page=1`. La sortie `forgejo-commit-status-snapshot-v1` lie le dépôt, le SHA et les contextes renvoyés.

Les états sont des observations Forgejo. `success` ne devient ni verdict indépendant ni autorisation de merge. L’objet vide v17 est émis comme `no_statuses`, jamais `success`; un faux succès sans contextes, un dépôt ou SHA incohérent, un état invalide et des compteurs incohérents sont refusés. Une page incomplète ou un `X-Total-Count` absent/différent devient `inconclusive`. Le champ `current_pr_head_checked=false` rappelle que l’appelant doit relire la PR et lier lui-même le SHA à la tête courante.

## Vérifications

Le test route a d’abord échoué sur le refus de la nouvelle route exacte; la sortie est conservée dans `red-route-policy.log`. Après le changement, `cargo test --locked --offline --bin forgejo-reconcile-local` passe **126/126**. Le test de contrat HTTP démarre un serveur loopback synthétique, vérifie l’URL exacte et `X-Total-Count`; il ne constitue pas un run Forgejo live.

La suite `cargo test --locked --offline --all-targets -- --test-threads=1` passe **545 réussis, 0 échec, 76 ignorés sur 34 groupes**. Les tests ignorés ne sont pas exécutés par cette commande. Les logs de la [suite complète](all-targets-final.log) et des [126 tests du binaire](bin-tests.log) sont conservés. `cargo fmt --check` et `git diff --check` passent. Le premier Clippy strict a trouvé deux lints dans `src/webhook/reconcile.rs` (`explicit_auto_deref`, `if_same_then_else`) après les deux allowances historiques du dépôt; le [Clippy du binaire](clippy-with-existing-allows.log) passe avec les quatre allowances nommées. Les sorties sont jointes.

`cargo run --quiet --locked --offline --bin forgejo-reconcile-local -- --capabilities` expose l’opération, le chemin, le caractère consultatif et les limites de liaison à la PR. `cargo metadata` et `scripts/verify_cargo_license_state.py` confirment toujours `license=null`, `license_file=null`, `publish=[]`, `no_license_selected`, droits `not_assessed`, `NOASSERTION`.

Le manifeste local du dossier passe **16/16**. Le journal global append-only conserve ses entrées historiques et ses empreintes courantes sont vérifiées pour le code, les trois documents actualisés, le manifeste local et les artefacts de ce lot; les sorties détaillées sont dans `manifest-check.txt` et `global-manifest-check.txt`.

## Sources et portée

Forgejo v17.0.0 est audité au commit officiel `0d791421f85ca13a63b665053d81ea765fbbc571` dans le scratch globalement modifié `/tmp/forgejo-v17-base-cas-poc`. Les fichiers `routers/api/v1/repo/status.go`, `modules/structs/status.go` et `services/convert/status.go` sont inchangés contre ce commit; leurs SHA-256 sont dans `source-sha256.txt`. `GetCombinedCommitStatusByRef` renvoie un `CombinedStatus{}` HTTP 200 quand il n’y a pas de statuts; sinon il utilise `GetLatestCommitStatus`, pose `X-Total-Count` et appelle `ToCombinedStatus`. `ToCombinedStatus` calcule un état à partir des entrées Forgejo, sans attester l’exécution indépendante des commandes de CI.

Le worktree produit était déjà fortement modifié à `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`; aucun reset, commit, publication ou accès externe n’a eu lieu. Aucun secret, LLM, AgentOS, instance Forgejo réelle ou coût nouveau n’a été utilisé dans ce lot. Aucun code Forgejo n’est copié.

## Limites et suite

Ce CLI n’intègre pas ces résultats au contexte de mission ni au gate. Il n’inspecte ni la PR courante, ni sa base, les exigences, la politique, les artefacts ou l’environnement; le statut peut avoir été écrit par un utilisateur ayant accès au dépôt. La pagination au-delà de 50 n’est pas suivie. Ce test ne démontre ni la validité des checks, ni le parcours LLM, ni une amélioration de valeur. La suite enregistrée au checkpoint reste l’audit des chemins de mutation de branches avec l’identité réelle du mergeur et des writers Git directs; la comparaison Forgejo+agent+CI attend toujours un runtime et un plafond de coût fiables.
