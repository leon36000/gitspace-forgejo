# CONTINUATION-421 — du retry issue corrigé au readback API

Date locale : 2026-10-28 (America/Montreal). Dépôt : branche `local/WEBHOOK-005-canonical-links`, HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`; worktree fortement modifié. Reçu limité au scénario sélectionné.

## Question vérifiée

CONTINUATION-420 avait prouvé le refus d’un `issue_comment` signé où `is_pull=false` coexiste avec `pull_request`, puis le retry du même ID après correction. Il restait à vérifier que l’événement durable corrigé traversait le CLI de réconciliation sans changer de route ni faire du texte du webhook l’état courant.

Le même test de processus exécute maintenant le parcours complet :

1. Le vrai `forgejo-webhook-local` reçoit le corps signé synthétique contradictoire. HTTP 400 `event_payload_invalid`, non réessayable; le marqueur privé n’est pas renvoyé et l’inbox est vide.
2. Le test retire `pull_request` et redélivre sous le même delivery ID. HTTP 202, `duplicate=false`, séquence 1; le journal ne contient que le payload corrigé.
3. Le vrai binaire `forgejo-reconcile-local` lit cette livraison. Une fixture HTTP loopback observe exactement `GET /api/v1/repos/forge-admin/agent-forge-core/issues/14`, puis `GET /api/v1/repos/forge-admin/agent-forge-core/issues/comments/24`. Aucune requête n’envoie `Authorization`.
4. Le résultat structuré est `current_state_read`, ressource `issue_comment`; l’issue est `closed`, le commentaire est `present`, l’URL de l’état parent vient de la configuration du client et le corps du commentaire vient de l’API courante, étiqueté `untrusted_user_content`. La sortie ne contient pas le marqueur du corps webhook. L’inbox reste octet-identique après le readback.

## Commande et résultat

```text
python3 evidence/continuation-421-corrected-issue-comment-to-api-readback-001/run_selected_test.py
```

Le wrapper appelle `scripts/run_webhook_reconcile_cli_e2e.py --scenario actual_receiver_refuses_issue_comment_marked_non_pull_with_pr_parent_then_accepts_same_delivery`. Cargo compile hors-ligne avec `--locked`, puis le binaire du test est exécuté dans Bubblewrap `--unshare-net --unshare-pid`. Résultat de `focused.log` : **1 passed, 0 failed, 0 ignored, 203 filtered out**, durée du scénario 0,09 s.

Le runner et le test partagent un namespace sans réseau externe; le scénario conserve un listener loopback privé pour les seuls services de test. Le binaire du CLI est copié en lecture seule dans ce namespace et n’envoie pas de credential. Les limites et montages sont détaillés dans `environment.txt` et la ligne de commande complète dans `focused.log`.

## Vérifications de l’arbre courant

Après formatage : `cargo fmt --all -- --check`, `git diff --check`, parse AST Python du wrapper/runner et `python3 scripts/verify_cargo_license_state.py` passent. La garde conserve `license=null`, `license_file=null`, `publish=[]`, `no_license_selected`, `redistribution_rights=not_assessed` et SPDX `NOASSERTION`. Aucun run `all-targets` n’a été exécuté dans ce lot.

## Portée et limites

Le changement est un test composé et sa traçabilité; aucun défaut runtime n’a été constaté ni corrigé. Les corps HMAC, le secret et les réponses API sont synthétiques, et l’API est une fixture HTTP loopback. Ce test ne prouve pas l’émission d’un webhook natif par une instance Forgejo, l’authenticité d’un événement après journalisation, un LLM/AgentOS, une PR ou une fusion. Il compose les processus receiver et CLI pour un seul cas (`n=1`) et ne mesure aucun gain général.

Les manifestes locaux/worktree/projet relient les fichiers de preuve, les quatre documents canoniques et le digest courant de l’index worktree; leurs préfixes append-only sont vérifiés.
