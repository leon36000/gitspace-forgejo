# WEBHOOK-025 — manifeste de fusion et E2E du descripteur de credential

Date : 23 octobre 2026. Worktree local observé à `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`; état Git déjà très modifié avant cette tranche. Aucun commit, push ou changement de production.

## Changements

- Le readback post-merge compare les entrées Git de la racine du commit candidat et du commit de fusion. Les deux arbres sont paginés, bornés et validés avant comparaison. Un manifeste SHA-256 lié au candidat et au commit de fusion est retourné seulement si les entrées correspondent; sinon le verdict est `inconclusive`.
- Le verdict d’un POST 4xx est `rejected` uniquement quand le readback confirme `merged: false`. Un état fusionné dont les bindings diffèrent ou manquent n’est plus présenté comme un refus certain.
- Forgejo v17 peut sérialiser une racine vide avec `tree: null`. Le lecteur accepte cette forme uniquement pour un arbre de zéro entrée, page 1, non tronqué. Le cas `total_count > 0` avec `tree: null` échoue fermé.
- Le client de réconciliation HTTP a une politique explicite GET-only, sans corps, et limitée aux routes d’issue/PR du dépôt configuré. Les tests vérifient que POST et `/issues/{n}/labels` sont refusés avant toute connexion.
- Le test de processus ajouté journalise une livraison signée via le vrai receiver, lance le vrai binaire `forgejo-reconcile-local`, passe un jeton synthétique par `--host-credential-fd`, vérifie l’en-tête Authorization et confirme que le jeton n’apparaît pas dans stdout/stderr.

## Vérifications fraîches

- `cargo test --offline --locked --all-targets -- --test-threads=4` : 32 groupes, 360 réussis, 0 échec, 13 ignorés. Les E2E marqués ignorés sont lancés séparément par leur runner isolé.
- `python3 scripts/run_webhook_reconcile_cli_e2e.py` : six tests de processus réussis sous `bwrap --unshare-net` : receiver/journal, credential FD, état PR, upstream interdit, retries bornés, reprise après interruption.
- TDD arbre vide : test rouge avant le changement (`exit=101`, `InvalidResponse` sur `tree: null`, `total_count: 0`), puis vert après le changement (1/1). Les réponses `tree: null` avec compte non nul sont également refusées par le même test.
- `cargo clippy --offline --locked --all-targets -- -A clippy::too_many_arguments -A clippy::bool_assert_comparison -A clippy::manual_div_ceil` : exit 0 avec les allowances historiques du paquet.
- `rustfmt --edition 2024 --check src/bin/forgejo-reconcile-local.rs` et `git diff --check -- src/bin/forgejo-reconcile-local.rs tests/webhook_receiver_process_e2e.rs scripts/run_webhook_reconcile_cli_e2e.py` : exit 0.
- `rustfmt --edition 2024 --check tests/webhook_receiver_process_e2e.rs` demeure non nul à cause de trois blocs préexistants dans le test PR, aux lignes 809, 848 et 874; la sortie exacte est conservée. Les nouvelles lignes ne figurent pas dans ces écarts.

Commandes et sorties : `webhook-025-all-targets.log`, `webhook-025-process-e2e.log`, `webhook-025-empty-tree-red.log`, `webhook-025-empty-tree-green.log`, `webhook-025-clippy.log`, `webhook-025-rustfmt-*.log`. La revue de l’API Forgejo v17 est consignée dans `webhook-025-forgejo-v17-review.md`.

## Essai live avec Forgejo v17.0.0

Après l’intégration, le harnais existant `scripts/run_forgejo_live_pr_merge_e2e.py` a été exécuté avec le binaire extrait de l’image officielle `codeberg.org/forgejo/forgejo:17.0.0`, publiée dans la [documentation de téléchargement Forgejo](https://forgejo.org/download/). Digest d’image : `sha256:06d38310de496a10186d952bdad5ca764c153c8023ede8dfa0944c79a1dc5412`; version vérifiée : `17.0.0+gitea-1.22.0`; SHA-256 du binaire : `e5ec437cd6357e8d1b4580cb47ab1daa58a32b33971a63db63446a535d33adb7`.

Le harnais a créé une instance fraîche dans `/tmp/webhook-025-forgejo-v17-live-001`, écoutant seulement sur `127.0.0.1:3130`, puis l’a arrêtée; le port était libre après la sortie. Le cas réel a créé issue #1 et PR #2 dans le bac à sable `forge-admin/agent-forge-core`, testé depuis la base `3122707b8bb93019690c0a37a49f2c7fb3bdd6fb` et le candidat `e09e79fa5c05330e8e42fb529862e1b3ef83e01f`. L’acceptance rouge échouait sur cinq cas; après le changement déterministe, l’acceptance verte rapporte 9/9. Une revue d’un compte distinct et la protection `main` ont été exigées.

Le gate Rust a retourné `verdict=merged`, HTTP 200, `merge_authorized=true`; le commit de fusion et la nouvelle tête `main` sont `c13f89767158d8e4ca7e0bfbec8d1b0cfdfba67b`. Les empreintes de tree du candidat et du commit de fusion sont toutes deux `9a3ab0f1a5a3ee19a5c318f32b55558076beb9b1bb01548bfc34732c1a46f1d4`. Le readback Forgejo post-merge montre l’issue fermée. Le harnais a également refusé le merge sans approbation, le merge d’un compte non allowlisté, un statut d’exécutant trompeur et un candidat périmé.

Ce run valide le nouveau binding contre le vrai API Forgejo v17 et la fusion contrôlée dans cette instance jetable, mais il utilise `deterministic local test adapter; no LLM configured or invoked`. Il ne compose donc pas ce gate modifié avec un LLM/AgentOS; le parcours Claude documenté dans les checkpoints antérieurs reste une preuve distincte. Les readbacks pré/post, le résultat du gate et le build sont conservés dans `webhook-025-live-*.json` et `webhook-025-live-cargo-build.log`. L’état de test privé sous `/tmp/webhook-025-forgejo-v17-live-001` contient les artefacts synthétiques et reste conservé pour reprise/revue.

## Limites

Les six E2E de processus utilisent un fixture HTTP local synthétique, pas une instance Forgejo réelle. La revue Forgejo v17 est une lecture du code officiel épinglé, pas un essai live. Aucun modèle LLM, AgentOS, RAG ou moteur de mémoire n’a été appelé dans ce lot. Le processus Claude déjà présent (PID 1947069, CLI 2.1.310) était toujours marqué `active=true`, itération 1/6 au relevé sûr de 23:06:15, mais son marqueur et ses deux rapports connus n’avaient pas bougé depuis 21:00–21:47; son modèle/effort courant n’est pas vérifié et le processus n’a pas été inspecté au-delà des champs autorisés, modifié ou relancé.

Le pipe assure un transfert hors argv/environnement, mais n’isole pas un processus hostile du même UID et ne prouve pas le scope du token Forgejo. L’E2E ne couvre pas un token invalide au niveau processus. La comparaison des trees est un binding de version; elle ne remplace ni les tests indépendants ni l’autorité de merge. Aucun gain général de qualité, temps, intervention, tokens ou coût n’est déduit d’un parcours isolé.
