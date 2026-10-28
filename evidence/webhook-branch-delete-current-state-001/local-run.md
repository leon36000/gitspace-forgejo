# WEBHOOK-BRANCH-DELETE-CURRENT-STATE-001 — preuve locale

## Contrat vérifié

Forgejo v17.0.0 distingue son événement `delete` de `push`. Dans le checkout scratch basé au commit officiel `0d791421f85ca13a63b665053d81ea765fbbc571`, `services/webhook/notifier.go::webhookNotifier.DeleteRef` émet `HookEventDelete` avec un `api.DeletePayload`; `modules/structs/hook.go::DeletePayload` expose `ref` et `ref_type`. Les deux fichiers concernés sont inchangés par les POC du scratch. Ce constat provient du code source; ce lot n’a pas provoqué une suppression dans une instance Forgejo en fonctionnement.

Le test-first `branch_delete_notification_reads_current_ref_before_reporting_presence` échoue avant le changement car le reconciler retourne `UnsupportedEvent`. Le receiver reconnaît maintenant `delete` seulement pour une ref de branche valide `refs/heads/...` et lie la livraison HMAC à cette branche. Les tags, les refs incohérentes et `ref_type` absent sont refusés avant append. À la réconciliation, la branche est relue par l’API : un snapshot actuel présent l’emporte sur la notification historique; `404` reste une erreur source inconclusive, jamais un état canonique « supprimé ».

Les tests processus passent par le vrai receiver signé, son journal durable et le vrai `forgejo-reconcile-local`, dans Bubblewrap avec réseau et PID namespace isolés. Une fixture HTTP représente un snapshot de branche recréée après la suppression; le CLI renvoie la tête courante, n’envoie pas d’Authorization et ne réécrit pas l’inbox. Une seconde fixture renvoie 404 : le CLI sort 1 avec `inconclusive/resource_not_found`, sans snapshot canonique, sans recopier le message privé et sans modifier l’inbox. Le test vérifie également que `/forgejo/capabilities` annonce `delete`. Un premier run de ce test de capacités a détecté l’ancienne liste JSON codée en dur; l’attente a été mise à jour puis le test a passé.

## Vérifications finales

- `webhook_reconcile_contract`: 32 réussis, 0 échec.
- `webhook_handler_contract`: 15 réussis, 0 échec.
- `webhook_contract`: 7 réussis, 0 échec.
- `webhook_http_contract`: 6 réussis, 0 échec.
- contrat processus des capacités: 1 réussi, 0 échec.
- parcours receiver → journal → CLI sous Bubblewrap pour snapshot courant présent: 1 réussi, 0 échec.
- parcours receiver → journal → CLI sous Bubblewrap pour 404: 1 refus inconclusif attendu, 0 test échoué.
- régression adjacente Host dupliqué sur le receiver actuel: 1 réussi, 0 échec.
- garde d’état Cargo: readback vérifié; ses 7 tests passent.
- `cargo fmt --all -- --check`, `git diff --check` et `py_compile` du runner passent.

Cargo conserve `license=null`, `license_file=null`, `publish=[]`, `no_license_selected` et `redistribution_rights=not_assessed`; aucun choix de licence ni de droits n’est fait. Aucun modèle, service externe, nouveau coût, Forgejo de production ou source GitSpace n’est utilisé. Aucun code Forgejo n’est copié.

## Portée et limites

Le payload de suppression du test processus est synthétique, structuré à partir des types Forgejo v17.0.0 inspectés; l’émission d’un webhook `delete` par Forgejo vivant et sa réconciliation vers un vrai `404` ne sont pas démontrées dans cette tranche. La réponse `404` reste ambiguë entre branche absente et accès masqué. Les suites complètes Rust/Python, le parcours LLM/AgentOS et la comparaison contrôlée ne sont pas couverts par ces tests ciblés. Le projet reste incomplet.

Commandes, versions, hashes, readback Cargo et sorties sont conservés dans ce dossier.

Une preuve séparée antérieure pour le test Host dupliqué conserve un hash de l’ancien fichier complet `tests/webhook_receiver_process_e2e.rs`. Cette tranche a ajouté des scénarios au même fichier; le manifeste local ancien ne correspond donc plus à son hash de source. Le test Host dupliqué a été relancé sur l’arbre courant et son résultat est conservé ici; les artefacts historiques n’ont pas été réécrits.
