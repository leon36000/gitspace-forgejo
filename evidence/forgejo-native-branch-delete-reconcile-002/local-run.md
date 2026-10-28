# WEBHOOK-BRANCH-DELETE-NATIVE-E2E-002

Date: 25 octobre 2026. Test local uniquement; Forgejo, dépôt, Git client, receiver et reconciler sont jetables et restent sur la machine.

## Question et résultat

Forgejo v17 Gitea webhooks transmet-il une vraie suppression de branche au receiver, et une notification tardive se réconcilie-t-elle avec la branche courante?

Le premier run (`forgejo-native-branch-delete-reconcile-001`) a échoué à bon escient : Forgejo a tenté le delivery `delete` avec `ref_type=branch`, mais le receiver a répondu HTTP 400 `event_payload_invalid`. Le `hook_task` local conserve `payload_content.ref=refs/heads/agent/native-delete-proof` et `request_content.body.ref=agent/native-delete-proof`. `first-failure-observation.json` conserve uniquement ces champs non secrets, le statut/code de refus et l’empreinte SHA-256 de la DB locale; aucun token, secret ou HMAC n’est inclus.

La cause a été confirmée dans le code officiel Forgejo v17.0.0 : `services/webhook/default.go::defaultHandler.NewRequest` appelle `substituteRefShortName` pour les hooks Gitea `create`/`delete`; ce helper remplace `ref` par `git.RefName(ref).ShortName()`. Le contrat productif a été corrigé pour traiter `delete.ref` comme le nom littéral d’une branche et valider `ref_type=branch`. Le reconciler réutilise ensuite le nom pour lire le snapshot Forgejo.

Le run 002 a créé une branche avec Git HTTP, envoyé `git push origin --delete agent/native-delete-proof` puis vérifié que `git ls-remote --heads` ne la retournait plus. Forgejo a émis un delivery natif `delete`, son hook task l’a lié à l’UUID observé et a enregistré HTTP 202 du receiver. Le runner a recréé la branche avant de réconcilier l’ancien delivery. Le lecteur non-admin a confirmé via l’API Forgejo la tête `96f58c94395b1b5c0f6a2826749697c5d99f193a`; le CLI a renvoyé `current_state_read`, le même SHA, et `upstream_read_attempts=1`. La lecture n’a pas modifié l’inbox. Le hook a été désactivé et relu; Forgejo et le receiver se sont arrêtés sans listener résiduel.

## Environnement et provenance

- Forgejo: `17.0.0+gitea-1.22.0`, Go `1.26.9`, binaire `/tmp/forgejo-17.0.0`, SHA-256 `e5ec437cd6357e8d1b4580cb47ab1daa58a32b33971a63db63446a535d33adb7`.
- Source Forgejo étudiée: `https://codeberg.org/forgejo/forgejo.git`, commit `0d791421f85ca13a63b665053d81ea765fbbc571`; les quatre fichiers référencés dans `forgejo-source-reference.txt` sont propres au diff du scratch. La provenance ne lie pas le binaire téléchargé/local à ce commit de build.
- Build des binaires produit: `cargo build --offline --locked` dans Bubblewrap `0.9.0`, avec namespaces réseau et PID séparés, environnement assaini et sources/registre/toolchain en lecture seule. Rust/Cargo `1.95.0`; les hashes des binaires, toolchain, source et registre sont dans `result.json`.
- Dépôt `forge-admin/agent-forge-core` privé sur `127.0.0.1:3130`; le lecteur avait `pull=true`, `push=false`. Seul le callback webhook était autorisé vers `127.0.0.1`; aucun service distant n’a été contacté. Tokens et secret webhook restaient en mémoire/fichiers privés, les credentials du reconciler passaient par un pipe FD privé et n’ont pas été enregistrés.
- L’état du run 002 est conservé dans `/tmp/forgejo-native-branch-delete-20261025-002`; l’essai 001 est conservé dans `/tmp/forgejo-native-branch-delete-20261025-001`. Aucun dossier ou journal n’a été supprimé.

## Vérifications

- `cargo test --offline --locked --test webhook_handler_contract`: 15 réussis.
- `cargo test --offline --locked --test webhook_http_contract`: 6 réussis.
- `cargo test --offline --locked --test webhook_reconcile_contract`: 32 réussis.
- `python3 -m unittest discover -s tests -p 'test_forgejo_emitted_webhook_e2e.py' -q`: 54 réussis.
- `scripts/run_webhook_reconcile_cli_e2e.py --scenario actual_reconcile_cli_process_reads_current_branch_after_native_delete_notification`: 1/1, sous Bubblewrap `--unshare-net --unshare-pid`.
- `scripts/run_webhook_reconcile_cli_e2e.py --scenario actual_reconcile_cli_process_keeps_native_delete_404_inconclusive`: 1/1, mêmes namespaces. Le 404 demeure inconclusif; aucune suppression canonique n’est publiée.
- `scripts/run_forgejo_native_branch_delete_webhook_reconcile.py` contre le Forgejo local ci-dessus: `verdict=passed`, un vrai delivery natif, une réconciliation courante; hook arrêté, listeners vérifiés absents.

Les sorties machine, payload webhook expurgé, readback de branche, hook tasks, résultat et hashes de source sont fournis dans ce dossier. `SHA256SUMS` couvre les artefacts du dossier.

Après ce run, le texte de l’action HTTP pour l’événement `unsupported_event` a été rendu cohérent avec `delete` et son format de branche courte; le commentaire Rust a également été précisé. Les tests handler 15/15, HTTP 6/6, reconciler 32/32, rustfmt et whitespace ont été relancés sur cet état. Le parcours Forgejo réussi précède seulement cette modification de texte/doc, sans changement du chemin réussi; `result.json` donne les hashes exacts du binaire et du source au moment du run.

## Portée et limites

Cette preuve couvre un seul dépôt et une branche, sans modèle, AgentOS, RAG, PR, vérificateur indépendant ni autorité de fusion. Elle démontre l’interopérabilité du webhook `delete` natif et la relecture de l’état courant; elle ne mesure pas la valeur générale de la plateforme. Le commit de build du binaire Forgejo n’est pas établi, l’instance n’est pas de production, et un 404 ne distingue pas suppression et visibilité privée. Aucun code Forgejo ou GitSpace n’a été copié dans le produit; aucune licence produit n’a été choisie.
