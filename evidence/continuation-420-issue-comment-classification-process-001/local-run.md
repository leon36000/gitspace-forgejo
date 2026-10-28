# CONTINUATION-420 — refus processus d’une classification issue/PR contradictoire

Date d’exécution : 2026-10-28 (America/Montreal). Dépôt local : branche `local/WEBHOOK-005-canonical-links`, HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`; worktree fortement modifié. Ce reçu ne suppose pas que les fichiers non suivis appartiennent tous à ce lot.

## Lacune et changement

Le contrat isolé vérifiait déjà la cohérence de `issue_comment.is_pull` et de `pull_request`, mais la matrice ne comportait pas le cas contradictoire sur le véritable processus receiver. Une confusion ici pourrait router un commentaire de PR par les routes d’issue. Le nouveau scénario `actual_receiver_refuses_issue_comment_marked_non_pull_with_pr_parent_then_accepts_same_delivery` ajoute ce refus au processus; son nom est ajouté à l’allowlist du runner isolé. Le validateur/runtime n’a pas changé.

Le test envoie une livraison signée HMAC synthétique avec `is_pull=false` et un objet `pull_request`. Le receiver répond HTTP 400 `event_payload_invalid`, non réessayable; le marqueur du commentaire n’apparaît pas dans la réponse et le journal reste vide. Le test retire ensuite `pull_request` et redélivre le même delivery ID : HTTP 202, `duplicate=false`, séquence 1. Le readback contient un seul événement corrigé, `is_pull=false`, sans parent PR.

## Exécution et résultat

Commande d’orchestration :

```text
python3 evidence/continuation-420-issue-comment-classification-process-001/run_selected_test.py
```

Le wrapper utilise `scripts/run_webhook_reconcile_cli_e2e.py --scenario actual_receiver_refuses_issue_comment_marked_non_pull_with_pr_parent_then_accepts_same_delivery`. Le runner compile hors-ligne avec Cargo puis exécute le binaire dans Bubblewrap avec `--unshare-net --unshare-pid`, environnement vidé, répertoire source en lecture seule et `/tmp`, `/build` et `/artifacts` temporaires. Le test garde uniquement son listener loopback. Résultat du reçu `focused.log` : **1 passed, 0 failed, 0 ignored, 203 filtered out**, test en 0,06 s. Les contraintes de ressources et les montages exacts figurent dans ce journal.

Deux premiers essais du wrapper se sont arrêtés avant Cargo à cause d’un chemin racine relatif incorrect, puis d’un `sys.path` incomplet; aucun test produit n’a échoué ou été exécuté par ces deux essais. Les corrections et l’ordre sont décrits dans `harness-attempts.log`.

Vérifications finales du workspace après normalisation Cargo : `cargo fmt --all -- --check` (code 0), `git diff --check` (code 0), parse AST Python du runner et wrapper (PASS), `python3 scripts/verify_cargo_license_state.py` (PASS : `license=null`, `license_file=null`, `publish=[]`, `no_license_selected`, droits `not_assessed`, SPDX `NOASSERTION`). Aucun run complet `all-targets` n’a été répété.

## Limites et traçabilité

Ce test utilise un HMAC et un payload synthétiques, le receiver local, et un port loopback privé au namespace. Il ne démontre pas un webhook Forgejo live, la réconciliation CLI, une PR, une fusion, un LLM ni une intégration AgentOS. Il ne prouve pas une amélioration générale; il couvre une seule forme contradictoire au niveau processus.

Le fichier `tests/webhook_receiver_process_e2e.rs` était déjà non suivi. Un appel `rustfmt` isolé l’a formaté sans passer l’édition du workspace; `cargo fmt --all` a ensuite restauré le format attendu par le workspace et le test final ci-dessus a été relancé sur cet état. Aucune copie pré-normalisation n’a été trouvée, donc l’empreinte finale seule est disponible et la portée exacte du changement de formatage par rapport à cette copie n’est pas vérifiable. Aucun fichier n’a été remis à zéro ni supprimé.

Le dernier prévol réseau GitLab depuis ce worker échoue à la résolution DNS de `pc4.tail0c55ff.ts.net`. Aucun courriel n’a été relu, aucun ancien credential n’a été utilisé, et aucun accès distant, `ls-remote`, commit ou push n’a été effectué. Le checkpoint documente qu’un credential neuf, limité au dépôt, et une route PC4 joignable sont nécessaires.
