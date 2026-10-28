# CONTINUATION-154 — un événement `deleted` ne remplace pas l’état API courant

Date : 26 octobre 2026. Worktree `.runtime/webhook-005-workspace`, HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. Le worktree reste fortement modifié; aucun reset, commit ou publication.

## Scénario

Le test processus `actual_reconcile_cli_process_uses_present_comment_readback_after_deleted_webhook` fait accepter par le vrai receiver une livraison HMAC `issue_comment/deleted`. Le vrai `forgejo-reconcile-local` relit ensuite le parent et le commentaire par une fixture HTTP loopback; les deux réponses sont `200` et la réponse du commentaire contient encore l’objet courant. Sous le runner Bubblewrap `--unshare-net --unshare-pid`, le test ciblé termine **1 réussi, 0 échec, 121 filtrés**.

Le résultat structuré est `current_state_read`, le commentaire vaut `present`, et son body est celui de l’API, pas le texte de la notification. Le test vérifie les deux routes GET, zéro Authorization, aucun écho du corps de webhook non fiable et une inbox identique octet pour octet. Le `deleted` du webhook déclenche un readback; il ne force pas `absent` si l’API dit `present`.

Cette preuve complète CONTINUATION-153 côté adaptateur en exerciceant le scénario opposé à un 404, et met à jour la ligne « Suppression de commentaire seulement confirmée par le webhook » de la matrice. Le runtime n’a pas changé.

## Vérifications

- `python3 scripts/run_webhook_reconcile_cli_e2e.py --scenario actual_reconcile_cli_process_uses_present_comment_readback_after_deleted_webhook`: **exit 0**, 1/1.
- Rustfmt, parsing AST du runner, `git diff --check`, garde d’état de licence Cargo et vérification des manifestes append-only : réussis; logs ci-joints.

Versions, HEAD et empreintes des sources se trouvent dans `toolchain-and-head.txt` et `source-sha256.txt`.

## Limites

Une fixture locale et un seul cas; ce n’est pas une API Forgejo live, ni un essai de délai de propagation après un vrai DELETE. Le dossier live cité dans la matrice est une preuve antérieure à provenance de build incomplète; cette tranche ne la renforce pas. Aucun modèle/AgentOS, coût, credential réel, accès externe ou gain de productivité. Le projet reste incomplet.
