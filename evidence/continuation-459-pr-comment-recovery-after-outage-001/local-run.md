# CONTINUATION-459 — reprise de la même livraison après épuisement des retries

Date : 29 octobre 2026. Worktree : `/home/pc6/Documents/Codex/2026-10-21-mission-construire-une-forge-con-ue/.runtime/webhook-005-workspace`; branche `local/WEBHOOK-005-canonical-links`; HEAD de base `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. Le checkout est fortement modifié; aucune modification préexistante n’a été supprimée, réinitialisée ou commitée.

## Parcours et résultat

Le scénario `actual_reconcile_cli_process_replays_same_deleted_pr_comment_after_upstream_retry_exhaustion` fait accepter au vrai receiver Rust une seule livraison signée `issue_comment/deleted`, puis arrête le receiver. Le premier processus `forgejo-reconcile-local` lit le parent PR (200), le commentaire (404), puis épuise trois lectures fraîches du parent (503 avec `Retry-After: 0`). Il retourne `inconclusive/upstream_http_status`, `retryable=true`, `http_status=503`, cinq lectures amont et aucun `canonical_state`. Aucun Authorization ou corps sentinelle ne fuit; le journal reste identique et contient une seule clé durable pour le delivery.

Sans redémarrer le receiver ni réinsérer l’événement, un second processus reçoit le même chemin de journal et le même ID de livraison. La fixture répond parent 200, commentaire 404, parent frais 200. Le résultat est `current_state_read`, PR fermée/fusionnée et `comment.state=absent`. Aucun Authorization n’est envoyé; les marqueurs de corps sont absents des sorties. L’inbox est toujours byte-for-byte identique et son replay contient toujours une livraison/clé d’idempotence. Le test de processus passe **1/1** dans Bubblewrap avec namespaces réseau et PID privés.

Le test est explicitement `#[ignore]` dans Cargo parce que sa fixture réserve l’endpoint local fixe utilisé par le runner; le runner `scripts/run_webhook_reconcile_cli_e2e.py` l’exécute par nom exact. Une première compilation a révélé une mauvaise hypothèse de type du test; elle est conservée dans `process-e2e.log`, puis corrigée avant le résultat final dans `process-e2e-final.log`. Le runtime applicatif n’a pas changé.

`cargo test --locked --offline --all-targets -- --test-threads=1` passe **39 groupes, 793 réussis, 0 échec, 219 ignorés**. Les tests ignorés ne sont pas déclarés exécutés dans cette suite; le scénario ciblé est exécuté séparément. Les commandes, versions et résumé machine figurent dans ce dossier.

## Prévol de transfert GitLab PC4 en lecture seule

L’instance `root/gitspace-forgejo` (ID 4) est privée. Le connecteur GitLab PC4 déjà authentifié, version 19.5.1, donne lecture du dépôt. Le 29 octobre, la branche par défaut `codex/pc6-transfer-2026-10-28` est toujours protégée à `6c6ffa3d7d37d48b9d7877acb982d73a2c49a14f` et `can_push=false`; la branche de travail `codex/pc6-transfer-work-2026-10-28` est non protégée à `9ac877767e9b458a9cd911e67a10f87aeeaefbdb` et `can_push=true`. Le pipeline 328 sur cette tête est en échec. Ces métadonnées sans secret sont conservées dans `gitlab-readonly-preflight.json`.

Le transfert de nouvelles données demeure suspendu par [GITLAB-PC4-SECURITY-REVIEW-001](../gitlab-pc4-security-review-001/local-run.md) : 988 findings `generic-api-key` avec champs `Secret` non vides qui n’ont pas été classifiés, plus un jeton d’upload d’artefact exposé dans une trace; l’artefact était encore disponible à la date de cet audit, avec expiration annoncée le 11 novembre 2026. Aucune valeur de secret n’est reproduite ici. Il faut confirmer la révocation/rotation des clés, la protection ou l’expiration de la trace et de l’artefact, et la classification des findings avant un push qui pourrait relancer ces jobs. Ne pas réutiliser l’ancien PAT mentionné dans l’audit.

Cette reprise n’a consulté ni utilisé d’accès du courriel; le connecteur GitLab existant suffisait au prévol. Aucun fichier, branche, MR, pipeline, artefact ou réglage GitLab n’a été modifié. Le contenu courant de PC6 n’a pas été transféré; cette preuve ne vaut pas transfert complet.

## Portée et limites

La réponse API est une fixture HTTP loopback synthétique, `n=1`, pas Forgejo live. Le parcours prouve la reprise d’une notification durable pour ce cas, pas une récupération multi-hôte, une fusion autorisée, un agent LLM/AgentOS, ou un gain de productivité. Les comptes all-targets ne comprennent pas les tests ignorés; les nombres sont des tests exécutés, pas un pourcentage d’avancement. Cargo reste sans licence choisie (`license=null`, `license_file=null`, `publish=[]`, `no_license_selected`, droits de redistribution `not_assessed`, SPDX `NOASSERTION`).
