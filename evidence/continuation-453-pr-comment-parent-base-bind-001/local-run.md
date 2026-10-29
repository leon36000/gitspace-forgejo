# CONTINUATION-453 — refuser une base PR étrangère avant le GET du commentaire

Date : 29 octobre 2026. Répertoire de travail : worktree local fortement modifié, base Git `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`; les modifications préexistantes ont été conservées. Ce reçu couvre un correctif ciblé et ses tests, pas le transfert GitLab en attente.

## Lacune et changement

Une livraison Forgejo `issue_comment` peut représenter un commentaire dans la conversation d’une pull request. Le reconciler lisait d’abord la PR par son numéro, puis le commentaire. `current_pull_request_fields` finissait par valider `base.repo`, mais `current_comment_parent_snapshot` ne faisait auparavant que lier le dépôt de l’issue : une PR de base étrangère n’était donc rejetée qu’après le GET du commentaire.

Le test `pull_request_comment_parent_rejects_foreign_base_before_reading_comment` a été écrit d’abord contre le comportement précédent. Il observait bien `ResourceMismatch`, mais comptait deux lectures d’API au lieu d’une; l’extrait est dans `red-observed.txt` et signale explicitement qu’il est transcrit, pas la sortie brute de la première commande. Le correctif réutilise `validate_current_pull_request_base_repository` depuis `current_comment_parent_snapshot` pour refuser le parent avant la projection et avant toute lecture du commentaire.

## Vérifications et résultats

- Test contrat ciblé après correction : `pull_request_comment_parent_rejects_foreign_base_before_reading_comment` passe **1/1**.
- Cible complète `webhook_reconcile_contract` : **110 réussis, 0 échec**.
- Processus complet receiver → inbox durable → véritable exécutable `forgejo-reconcile-local`, sous Bubblewrap `--unshare-net --unshare-pid`, contre un serveur HTTP loopback de fixture : **1/1**, les 210 autres tests de cette cible étant filtrés. Le processus reçoit une PR dont `base.repo` est étranger et fait exactement un `GET /pulls/23`; aucun GET de commentaire ni en-tête Authorization, verdict `inconclusive/resource_mismatch`, aucun `canonical_state`, journal byte-for-byte inchangé. Le scénario est `actual_reconcile_cli_process_refuses_foreign_pr_base_before_reading_comment`.
- Suite verrouillée hors ligne `cargo test --locked --offline --all-targets -- --test-threads=1` : **39 cibles, 790 réussis, 0 échec, 213 ignorés**. Le test processus dédié est ignoré dans cette suite ordinaire et exécuté séparément par le runner Bubblewrap.
- `cargo fmt --all -- --check`, `git diff --check`, compilation syntaxique Python du runner, politique de licence Cargo et audit des liens locaux passent. L’audit vérifie 1 674 liens dans les cinq documents canoniques.
- Cargo reste `license=null`, `license_file=null`, `publish=[]`, `no_license_selected`, droits de redistribution `not_assessed`, SPDX `NOASSERTION`.
- Les manifestes append-only passent sur **14 085 chemins worktree** et **9 254 chemins projet**, sans cible manquante, divergente, actuelle non hachée ou dangereuse. Une seule ancienne ligne worktree legacy non hachée est signalée; elle n’est pas modifiée par cette tranche.

## Répétabilité et incident de test

Les commandes exactes sont dans `commands.txt`, les versions des outils dans `tool-versions.txt`, les sorties dans `all-targets.log`, `reconcile-contract.log`, `process-e2e-final.log`, `rustfmt.log`, `diff-check.log`, `runner-syntax.log`, `cargo-license-state.json` et `doc-links.json`. Le premier lancement du scénario processus a échoué à cause d’une assertion CRLF mal échappée dans le test lui-même; le littéral a été corrigé puis le scénario a passé 1/1. Le journal initial et le journal final sont conservés séparément.

## Limites

Le serveur HTTP est une fixture locale, pas une instance Forgejo native ni un service PC4. Ce test démontre l’ordre des requêtes et le refus du client Rust dans ce cas synthétique; il ne prouve ni l’authenticité de l’API, ni toutes les voies d’accès, ni un contrôle d’egress général du processus agent. Il n’y a eu aucun appel LLM/AgentOS, aucune fusion ni modification GitLab. Le parcours de mission et les critères de comparaison restent ouverts.

Le projet PC4 privé `root/gitspace-forgejo` contient déjà le transfert antérieur vérifié de 14 715 chemins candidats, mais ce manifeste n’est pas un export complet du checkout ni de l’historique Git. L’export excluait `.runtime`, `.claude`, archives/binaires, contenus non UTF-8 et 108 snapshots source; voir [TRANSFER-012](../gitlab-pc4-transfer-012/local-run.md) et [ACCESS-014](../gitlab-pc4-access-014/local-run.md). Le pipeline 328 du commit `9ac877767e9b458a9cd911e67a10f87aeeaefbdb` échoue à Gitleaks sur dix chaînes de jeton de test dont la révocation n’est pas établie. Le connecteur GitLab fournit l’accès sans courriel; aucune nouvelle écriture distante n’a été tentée pendant l’attente de la décision de l’utilisateur sur leur conservation ou leur révocation/rédaction.
