# CONTINUATION-455 — `draft` PR obligatoire avant lecture du commentaire

Date : 29 octobre 2026. Travail effectué dans le worktree local `/home/pc6/Documents/Codex/2026-10-21-mission-construire-une-forge-con-ue/.runtime/webhook-005-workspace`, base `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`, branche `local/WEBHOOK-005-canonical-links`. Le dépôt était déjà fortement modifié; aucune modification préexistante n’a été annulée, écrasée, commitée ou publiée.

## Lacune ciblée

CONTINUATION-454 a établi que l’ensemble du snapshot d’une PR parente est contrôlé avant l’API enfant du commentaire. Son processus refusait `head.sha` invalide; la fixture positive avait également révélé qu’un `draft` absent provoquait le refus du parent. Ce lot transforme cette seconde observation en contrat dédié : le GET PR courant sans booléen `draft` doit retourner `InvalidCurrentStateField("draft")` après une seule lecture, sans GET du commentaire.

Aucun code runtime n’a changé dans ce lot. Le contrat Rust et le scénario de processus sont des tests de non-régression autour du comportement installé dans CONTINUATION-454.

## Résultats observés

- Contrat ciblé `pull_request_comment_parent_rejects_missing_draft_before_reading_comment` : **1/1**.
- Contrat `webhook_reconcile_contract` : **112/112**, zéro échec.
- Processus isolé `actual_reconcile_cli_process_refuses_missing_pr_draft_before_reading_comment` : **1/1** dans le runner Bubblewrap avec namespaces réseau et PID privés. Receiver et inbox durables réels, binaire `forgejo-reconcile-local` réel, API loopback synthétique.
- Assertions processus : le receiver accepte la livraison HMAC de fixture; le CLI fait un seul `GET /pulls/24`, aucun `GET /issues/comments/916`, aucun en-tête Authorization; verdict `inconclusive/invalid_current_state`, champ `draft`, une lecture amont, aucun état canonique ni écho du corps webhook, inbox inchangée byte-for-byte.
- `cargo test --locked --offline --all-targets -- --test-threads=1` : **39 cibles, 792 réussis, 0 échec, 215 ignorés**. Les scénarios processus marqués `ignore` restent exécutés séparément via le runner.

La sortie du contrat est `reconcile-contract.log`, celle du processus est `process-e2e.log`, et les compteurs agrégés sont dans `test-summary.json`.

## Outils, compatibilité et politiques

Environnement : Python 3.12.3; Cargo 1.95.0; rustc 1.95.0; Git 2.43.0; Bubblewrap 0.9.0. Sorties : `tool-versions.txt`.

L’API testée est le GET de PR `GET /repos/{owner}/{repo}/pulls/{index}`. La référence officielle consignée dans `docs/source-inventory.md:286` fixe Forgejo v17.0.0 au commit `0d791421f85ca13a63b665053d81ea765fbbc571`; elle documente la route, `reqRepoReader` et le DTO PR. Aucun nouveau code Forgejo n’a été copié et aucune source upstream n’a été modifiée. La réponse de ce run est une fixture, pas un endpoint Forgejo live.

La vérification Cargo confirme `license=null`, `license_file=null`, `publish=[]`, `no_license_selected`, redistribution `not_assessed`, SPDX `NOASSERTION`; aucune licence produit n’est choisie. L’audit des cinq documents canoniques contrôle 1 682 liens locaux, sans cible manquante. Rustfmt, `git diff --check` et la compilation syntaxique du runner Python passent. Le runner met son cache Python hors du worktree. Les sorties de politique et de liens sont dans `cargo-license-state.json` et `doc-links.json`.

Aucun AgentOS/LLM n’a été intégré ou appelé. La réévaluation locale OPSYS existante conclut qu’aucune interface runtime autorisée compatible avec `AgentRuntimeV1` n’est démontrée et conserve `FixtureRuntime` comme adaptateur de test : [OPSYS-RUNTIME-INTERFACE-REASSESSMENT-001](../opsys-runtime-interface-reassessment-001/local-run.md). Ce lot n’a engagé aucun coût, consulté aucun courriel, utilisé GitLab ou un service externe, ni modifié la production.

## Intégrité et limites

Les commandes sont dans `commands.txt`; les hashes des six fichiers sources/documents touchés sont dans `source-sha256.txt`. Les manifestes append-only worktree, projet et paquet sont vérifiés par `append_manifests.py` : **14 125 chemins uniques** dans le worktree, **9 294** dans le projet, sans entrée manquante, divergente, actuelle non hachée ou dangereuse. Un registre historique non haché reste signalé dans l’index worktree; l’index projet n’en contient aucun. Les préfixes préexistants sont dans `manifest-prefix-before.json`.

Portée `n=1` pour le nouveau processus et un champ manquant. Cette preuve n’établit ni la réponse d’une instance Forgejo live, ni les refus de chaque autre champ du snapshot, ni une isolation générale multi-hôte, AgentOS/LLM ou gain comparatif. Prochaine tranche : si le GET frais du parent après un 404 de commentaire supprimé renvoie un champ PR obligatoire invalide, vérifier en contrat et au processus que le client reste inconclusif et ne projette pas `comment.absent`.
