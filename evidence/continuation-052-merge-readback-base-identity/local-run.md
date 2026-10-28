# CONTINUATION-052 — lier la lecture post-fusion au dépôt configuré

Date : 26 octobre 2026. Worktree : `/home/pc6/Documents/Codex/2026-10-21-mission-construire-une-forge-con-ue/.runtime/webhook-005-workspace`; HEAD de base `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`, branche `local/WEBHOOK-005-canonical-links`. Le dépôt reste fortement modifié; la correction est non commitée.

## Lacune démontrée

Le test préexistant `tests::merge_readback_refuses_divergent_commit_and_issue_state` échouait de façon isolée. `merge_readback_is_confirmed` liait la PR à son numéro, état, SHA de tête, branche de base, SHA de base, commit de fusion et issue, mais ne contrôlait pas `base.repo.full_name`. Une réponse avec un dépôt de base étranger — ou sans identité de dépôt — pouvait donc satisfaire les autres prédicats. La pré-admission utilise déjà `PullRequestSnapshotV1::from_forgejo_response`, qui exige l’identité du dépôt cible.

## Correction

Le verdict post-fusion reçoit le propriétaire et le dépôt du profil Forgejo sélectionné et exige `base.repo.full_name == "{owner}/{repository}"`. Ces valeurs traversent les chemins de fusion directe et de récupération d’un intent durable. Une absence, une identité étrangère ou une réponse mal formée rend le readback inconclusif. `--capabilities` annonce maintenant `postmerge_pull_request_identity`; les fixtures valides reflètent la réponse liée au dépôt attendue, tandis que le test de refus conserve les cas étranger et absent.

## Résultats

Après correction : le test `merge_readback_refuses_divergent_commit_and_issue_state` passe 1/1; la suite du binaire `forgejo-reconcile-local` passe 139/139; `cargo test --offline --locked --all-targets` passe 576/576 avec 0 échec et 85 ignorés en 34 cibles. Le log complet final est `all-targets-final.log`. `cargo fmt --all -- --check` et `git diff --check` passent.

Cargo metadata reste `license=null`, `license_file=null`, `publish=[]`; aucune licence produit n’a été choisie. Les versions d’outils, la sortie résumée et les empreintes sont jointes à ce dossier.

## Limites

Les résultats sont des tests Rust et fixtures HTTP locales, sans serveur Forgejo pour cette correction. Le contrôle lie le verdict final au dépôt annoncé par l’API; il ne rend pas atomique le POST de fusion et n’exclut pas les writers Forgejo externes. Aucun parcours LLM/AgentOS, service externe, donnée privée ou opération de production.
