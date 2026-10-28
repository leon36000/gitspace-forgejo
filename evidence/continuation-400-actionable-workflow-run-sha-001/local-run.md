# CONTINUATION-400 — conseil de correction pour un SHA de workflow run

Date : 28 octobre 2026. Worktree `/home/pc6/Documents/Codex/2026-10-21-mission-construire-une-forge-con-ue/.runtime/webhook-005-workspace`, branche `local/WEBHOOK-005-canonical-links`, HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. Le worktree était déjà fortement modifié; aucun changement antérieur n’a été réinitialisé, stage ou commité.

## Contrat et test d’abord

Le reconciler validait déjà que `run.commit_sha` est un SHA non nul de 40 ou 64 caractères hexadécimaux. En cas d’erreur, l’ingress renvoyait HTTP 400 `event_payload_invalid`, `details.field_path=payload.run.commit_sha`, mais son action tombait dans le conseil générique « inspect the event payload field ». Cette branche n’était ni une autorisation ni une correction du validateur; seul le message opérateur manquait.

Le test processus `actual_receiver_refuses_workflow_run_invalid_commit_sha_with_actionable_guidance_before_reserving_delivery` signe un événement `workflow_run_success` avec une fixture HMAC locale. Le premier run échoue exactement parce que l’action HTTP est générique. Il vérifie ensuite, après le changement, que le refus est non réessayable et précise le champ, que l’inbox ne change pas et reste vide, puis que le payload corrigé accepté avec le même delivery ID reçoit la séquence 1 et devient l’unique événement rejoué. Les reçus rouge/vert et cible sont fournis séparément dans ce dossier.

## Changement et vérification

`src/webhook/http.rs` donne désormais pour `run.commit_sha` une consigne de produire un identifiant hexadécimal non nul de 40 ou 64 caractères, de consulter le contrat de capacités et de redélivrer le même delivery ID. Aucun statut, règle d’admission, schéma, dépendance ni métadonnée de licence ne change.

Résultats après le formatage final :

```text
cargo test --offline --locked --test webhook_receiver_process_e2e -- --test-threads=1
43 passed; 0 failed; 148 ignored

cargo test --offline --locked --all-targets -- --test-threads=1
754 passed; 0 failed; 188 ignored; 38 test-result groups

cargo fmt --all -- --check
exit 0

git diff --check
exit 0
```

La garde Cargo produit `license=null`, `license_file=null`, `publish=[]`, `no_license_selected`, `redistribution_rights=not_assessed`, SPDX `NOASSERTION`; aucun choix de licence n’est fait. Outils : Cargo 1.95.0, rustc 1.95.0. Les journaux de la suite complète, du statut de licence, des versions et des empreintes sources sont dans ce dossier.

## Portée et limites

Le test traverse le vrai binaire receiver et son HTTP loopback, mais il utilise un payload/secret de fixture. Il ne démontre pas l’émission par Forgejo vivant ni la livraison de `workflow_run` native; aucune instance, aucun réseau externe, modèle, AgentOS, RAG ou mémoire n’est utilisé. Les tests marqués ignored ne sont pas exécutés par `--all-targets`. Les messages pour des champs tels `run.updated` et `run.index_in_repo` restent génériques et sont des lacunes distinctes. Le projet demeure incomplet.
