# CONTINUATION-422 — contrat local pour les notifications Forgejo Release

Date : 28 octobre 2026 (America/Montreal). Worktree : branche `local/WEBHOOK-005-canonical-links`, HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. Le dépôt était déjà fortement modifié; aucun fichier antérieur n’a été remplacé.

## Lacune et changement

Le runner `scripts/run_forgejo_live_webhook_reconcile_emitted.py` reconnaissait les sélecteurs de hook `issues` et `pull_request`, mais pas le groupe `release`, et son binder de tâches ignorait un `event_type=release`. Le réconciliateur Rust et le contrat webhook v9 couvrent déjà la lecture courante `GET /api/v1/repos/{owner}/{repository}/releases/{release_id}`.

Le runner sait maintenant résoudre le sélecteur `release`, associer une tâche Forgejo `release` à l’événement correspondant du journal, et valider un enregistrement `forgejo.webhook.v1.release`. Le parseur exige une enveloppe Forgejo v1, un delivery ID lié à sa clé d’idempotence, le dépôt attendu, une action `published`, `updated` ou `deleted`, et un ID strictement positif dans la plage `u64` du réconciliateur Rust. Il ne retourne que la séquence, le delivery ID, l’événement, l’action et l’ID Release. Tag, notes, assets et URL reçus du webhook ne deviennent pas canoniques.

La valeur par défaut de `run()` reste `issues` + `pull_request`; le lot ne crée pas encore de Release depuis l’API Forgejo et ne prouve pas l’émission native. Il s’agit du contrat et des liaisons de la couche de test live, pas d’un parcours Forgejo natif complet.

## TDD et résultats

Les tests rouges ont montré successivement : parseur Release absent; ID `2^64` accepté; sélecteur `release` rejeté; tâche de type `release` non liée au journal. Après corrections, les tests ciblés `ParseLiveReleaseDeliveryRecordTests`, `ReleaseWebhookBindingTests`, `HookReadbackTests` et `PushWebhookBindingTests` passent **11/11**. Le test de frontière confirme que `2^64−1` est accepté et `2^64` refusé. Détails de commandes : `commands.txt`; résumés sans corps de requête : `focused.log` et `tdd-red.log`.

Le garde de licence Cargo passe **12/12** tests. Un appel frais à Cargo confirme `license=null`, `license_file=null`, `publish=[]`, `decision=no_license_selected`, `redistribution_rights=not_assessed`, et le marqueur personnalisé `NOASSERTION`. Aucune licence n’est choisie et aucune conclusion juridique n’est faite.

La suite complète `tests/test_forgejo_emitted_webhook_e2e.py` a été exécutée : **171 tests, 16 échecs, 20 erreurs, 0 ignoré**. Aucun échec n’est dans les nouvelles classes Release. Les 36 cas en échec sont dans les validateurs existants de branches, push/PR batch, commentaires et projection de contenu; les messages indiquent des fixtures d’enveloppe/compte de lecture ou attentes d’erreur qui ne correspondent pas aux validateurs courants. Ils ne sont pas corrigés dans cette tranche et restent à trier. Voir `full-suite-summary.txt`.

AST Python et `git diff --check` passent. Les manifestes locaux/worktree/projet et leurs préfixes append-only sont vérifiés par le script fourni; les SHA-256 locaux sont dans `SHA256SUMS`.

## Limites et suite

`forgejo` et `go` ne sont pas disponibles dans PATH; aucun serveur Forgejo n’a été lancé ou contacté. Les fixtures de test ne démontrent pas un webhook natif, les routes et permissions d’une instance, une PR ou une fusion. AgentOS, Forge Code, RAG et mémoire n’ont pas été intégrés ou réévalués par ce lot. Aucun coût, accès distant ou changement de production.

Prochaine action : traiter séparément les 36 erreurs de la suite Python afin de rétablir une base de validation fiable, puis câbler une option de scénario qui crée une Release jetable, observe le webhook natif, valide sa tâche et réconcilie l’ID courant sur une instance Forgejo isolée lorsqu’un binaire local approprié est disponible.
