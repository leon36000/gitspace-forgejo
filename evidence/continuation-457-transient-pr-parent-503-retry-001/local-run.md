# CONTINUATION-457 — retry du parent PR frais après un 503

Date : 29 octobre 2026. Worktree : `/home/pc6/Documents/Codex/2026-10-21-mission-construire-une-forge-con-ue/.runtime/webhook-005-workspace`; branche `local/WEBHOOK-005-canonical-links`; HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. Le checkout était déjà fortement modifié (1 892 chemins modifiés/non suivis avant ce lot). Aucun changement préexistant n’a été annulé ou écrasé; aucun commit n’a été créé.

## Scénario et résultat

Le test ajouté `actual_reconcile_cli_process_retries_fresh_pr_parent_before_projecting_deleted_comment_absent` compose le vrai receiver Rust, son inbox durable, le binaire `forgejo-reconcile-local` et une fixture HTTP loopback. Pour un webhook signé `issue_comment/deleted` lié à une PR, la fixture renvoie successivement : parent PR initial valide, commentaire 404, parent PR frais 503 avec `Retry-After: 0`, puis parent PR frais 200 après le retry.

Sous Bubblewrap avec namespaces réseau et PID privés, le résultat est `current_state_read`, `upstream_read_attempts=4`, et l’état projeté vient du parent final (`closed`, fusionné, avec le SHA de fusion attendu). `comment.state=absent` n’est émis qu’après le readback réussi. Les quatre chemins GET sont dans l’ordre attendu, aucun Authorization n’est présent, aucun corps sentinelle 404/503 ni texte non fiable n’apparaît dans la sortie, et les octets de l’inbox restent identiques. Le scénario isolé passe **1/1**. Le test reste `#[ignore]` dans l’exécution Cargo ordinaire et est autorisé dans `scripts/run_webhook_reconcile_cli_e2e.py`.

La suite verrouillée et hors ligne `cargo test --locked --offline --all-targets -- --test-threads=1` passe **39 groupes, 793 réussis, 0 échec, 217 ignorés**. Le scénario processus ci-dessus a été exécuté séparément sous Bubblewrap; les tests `#[ignore]` ne sont pas comptés comme exécutés par `--all-targets`. `cargo fmt --all -- --check`, `git diff --check`, la syntaxe Python du runner et le garde de politique Cargo passent. Les journaux, le résumé machine, les versions, les commandes, les hashes sources et les reçus de manifestes sont dans ce dossier.

## Portée et limites

Ce lot n’a modifié que le test processus, son allowlist, les documents canoniques et les preuves; le runtime Rust n’a pas changé. Le cas valide un seul ordre de réponses synthétiques (`n=1`), pas une instance Forgejo live. Il ne vérifie pas l’épuisement des retries, la reprise après interruption de cette même séquence, une atomicité entre readback et fusion, un agent LLM/AgentOS, ni un gain comparatif. La licence produit reste non choisie et la publication Cargo désactivée.

Dans ce tour, le connecteur GitLab PC4 a seulement relu les métadonnées du projet privé. Aucun courriel n’a été ouvert et aucune écriture distante n’a été faite. Le transfert PC4 demeure suspendu par les détections Gitleaks et le jeton d’upload exposé décrits dans [GITLAB-PC4-SECURITY-REVIEW-001](../gitlab-pc4-security-review-001/local-run.md); ne pas réutiliser le PAT antérieur.

Prochaine tranche locale : épuiser les retries 503 sur le parent frais après le 404 commentaire et démontrer qu’aucune absence canonique n’est retournée sans readback parent réussi.
