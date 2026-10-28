# CONTINUATION-204-A2 — webhook d’équipe et readback Rust d’une PR d’organisation

Run local isolé du 27 octobre 2026 dans le worktree `webhook-005-workspace`, HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`, déjà fortement modifié. Le dépôt Forgejo d’origine reste externe au produit; aucun code Forgejo ou GitSpace n’est copié.

## Changement livré

`forgejo-reconcile-local` propose maintenant le profil strict `--reconcile-target-profile org-team-review-e2e`, limité au tuple compilé `http://127.0.0.1:3130`, propriétaire `forge-team-org`, dépôt `team-review-readback`. Il est défini dans un type distinct des profils de fusion; il ne figure pas dans la liste `local_test_merge_authority.target_profiles`. Le parseur refuse les profils inconnus, répétitions, ou combinaisons avec `--config` et `--target-profile`. La découverte passe de capacités schema/API v4 à v5 et annonce la cible comme lecture seule et non utilisable pour merger.

Le runner live crée une organisation privée et une équipe `reviewers` avec `repo.pulls=write`, ajoute un membre, crée un dépôt privé, une PR ouverte et un webhook limité à `pull_request` vers le receiver loopback. Après `POST team_reviewers`, Forgejo retourne 201 et le GET API courant retourne l’ID et le nom non vides de l’équipe. Le receiver Rust inscrit un seul `pull_request/review_requested`; le CLI Rust lit la livraison avec un credential du membre créé sous les scopes `read:user`, `read:repository`, `read:issue`.

La réponse Rust est `current_state_read`, liée au dépôt `forge-team-org/team-review-readback`, PR #1, état ouvert/non fusionné, SHA base `c7ebddc54b2c4ff540f9982c6387cadb7f1b01ee`, SHA tête `c985a05fb5d83b6d266c43cc79a57ab3b9c81921` et reviewers d’équipe exactement `[{"id":2,"name":"reviewers"}]`. Le journal inbox est byte-for-byte inchangé pendant la lecture. Le payload webhook ne fournit pas de champ `requested_team` dans ce run; l’affectation de l’équipe est attestée par l’API canonique courante, pas inférée de l’événement.

## Vérification

- Le nouveau test Rust est d’abord rouge sur le profil absent; après implémentation, les tests de parseur ciblés passent `6/6`.
- Le test de découverte du CLI et la suite du module de capacités passent; le module rapporte `3` passés et `2` ignorés pour l’occupation explicite du port fixe 3130.
- Les tests Python du validateur passent `6/6`; `py_compile`, `cargo fmt --all -- --check`, `git diff --check`, le vérificateur de métadonnées Cargo et ses `12` tests passent.
- Le builder E2E compile le receiver et le CLI dans Bubblewrap 0.9.0 avec réseau/PID non partagés, source et registry staged, `cargo build --offline --locked`. Les hashes binaires et versions sont dans `build-provenance.json`; le rapport de compilation est `cargo-build.log`.
- Forgejo live annonce `17.0.0+gitea-1.22.0`, Go 1.26.9. Le commit exact du binaire est inconnu. L’organisation est privée; les deux processus terminent avec code 0 et aucun listener ne subsiste sur le port 3130 ou celui du receiver.

Le premier run de ce lot avait échoué sur l’import du helper JSON, avant le démarrage de Forgejo; l’erreur est conservée dans [CONTINUATION-204-A1](../continuation-204-live-org-team-reviewer-rust-reconcile-001/failed-run.md). Il n’est pas fusionné au résultat final.

## Portée et limites

Cette preuve intègre une instance Forgejo locale, une requête webhook native, le receiver Rust et le CLI Rust, avec une PR d’organisation et une liste d’équipe non vide (`n=1`). Elle ne teste aucune mutation du CLI, l’interdiction effective d’un appel write par le token, une décision de vérification ou de fusion, le comportement post-merge, un agent/LLM réel ou un gain comparatif. La demande reviewer à l’équipe et la réconciliation sont deux opérations séparées; l’événement webhook n’identifie pas lui-même l’équipe dans son payload observé. Aucun AgentOS, RAG ou moteur de mémoire n’est impliqué. Aucun fournisseur, dépense, service externe, production, publication ou commit n’est impliqué.

Le projet reste incomplet : parcours LLM autorisé, critères et preuves indépendantes de fusion pour ce scénario, contrôles de refus cumulatifs, droits/licences complets et comparaison appariée restent ouverts. Les artefacts et commandes sont hashés localement; le reçu global suit les index append-only.
