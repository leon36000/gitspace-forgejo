# CONTINUATION-047 — refuser les OID nuls dans l’état courant d’une PR

Date : 26 octobre 2026. Worktree : `local/WEBHOOK-005-canonical-links`, HEAD de base `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`; worktree préexistant fortement modifié et préservé.

## Défaut constaté

`src/webhook/reconcile.rs::reconcile_inbox_event` vérifiait la longueur et l’alphabet hexadécimal de `head.sha`, `base.sha` et `merge_commit_sha`, mais la validation générique acceptait l’OID composé de zéros. Pour une PR déclarée fusionnée, cela permettait de produire un `ReconciledState` canonique avec un `merge_commit_sha` qui ne désigne aucun commit. Le même trou existait sur les références courantes de tête et de base.

Les tests TDD ont échoué avant correction à deux frontières. Le test de contrat a obtenu `Ok(ReconciledState { merged: Some(true), merge_commit_sha: Some("000…000") })` à la place de `InvalidCurrentStateField("merge_commit_sha")`. Le processus CLI réel a lui aussi échoué à l’assertion qui attendait un refus structuré. Les deux observations figurent dans la sortie de session; les commandes sont consignées ci-dessous.

## Correction

La réconciliation exige maintenant `valid_nonzero_commit_sha` pour les trois champs du snapshot d’une PR. La règle accepte toujours les longueurs SHA-1 et SHA-256 reconnues par le helper, mais refuse les chaînes de zéros. Elle ne change pas la sémantique de `push.before`/`push.after`, où l’OID nul représente légitimement la création ou la suppression d’une ref, ni celle de `create.sha`, qui avait déjà sa propre règle non nulle.

Tests ajoutés :

- `merged_pull_request_with_zero_merge_commit_sha_is_not_canonical` : OID nul de 40 et 64 caractères.
- `pull_request_reconciliation_rejects_zero_head_and_base_object_ids` : les deux champs et les deux longueurs.
- `actual_reconcile_cli_process_refuses_merged_pull_request_with_zero_merge_commit_sha` : le vrai receiver accepte une livraison HMAC fixture; le vrai CLI lit une réponse HTTP fixture `200`, retourne `inconclusive/invalid_current_state` sur `merge_commit_sha`, n’émet aucun `canonical_state` et laisse l’inbox identique octet pour octet.

## Résultats

- Suite `webhook_reconcile_contract` : **42 réussis, 0 échec, 0 ignoré**.
- Processus CLI sous Bubblewrap `--unshare-net` : **1 réussi, 0 échec**; 91 autres tests du binaire sont filtrés.
- `cargo fmt --all -- --check`, `git diff --check`, `py_compile` du runner E2E et le garde Cargo passent.
- Relecture `cargo metadata --locked --offline --no-deps` : `license=null`, `license_file=null`, `publish=[]`. Le marqueur demeure `no_license_selected`, `redistribution_rights=not_assessed`, SPDX personnalisé `NOASSERTION`; aucune licence n’est choisie.

Logs, commandes, versions, hashes source et manifestes de ce lot sont dans ce dossier.

## Portée et limites

Le serveur HTTP de la preuve processus est une fixture loopback synthétique; ce lot ne démarre pas Forgejo et ne vérifie pas l’existence réelle d’un OID renvoyé. Le validateur établit seulement qu’une valeur non nulle a la forme d’un identifiant commit SHA-1/SHA-256, pas qu’elle existe, qu’elle est atteignable ou qu’elle correspond au contenu fusionné. Aucun AgentOS/LLM, modèle, Forgejo de production, service externe, nouveau coût, publication ou changement Forgejo n’est impliqué. Le parcours LLM et la comparaison agent+CI contrôlée restent non démontrés.

Suite locale : examiner le confinement par branche d’un intent de fusion devenu inconclusif après un parent de base inattendu. La tentative actuelle reste non résolue et bloque sa propre reprise; il faut déterminer par un test si une autre PR vers la même branche peut tout de même lancer un POST, puis ne proposer une quarantaine durable que si le gap est reproduit. Le verrou coopératif ne remplace toujours pas une précondition CAS côté serveur ni ne bloque les writers externes.
