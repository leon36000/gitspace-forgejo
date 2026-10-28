# CONTINUATION-124 — refuser une ligne de review persistée et expliquer sa récupération

Date : 2026-10-26. Worktree de base : `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`, branche `local/WEBHOOK-005-canonical-links`, arbre fortement modifié. Aucun reset, commit, staging, service externe ou accès production.

## Défaut constaté

Le CLI de réconciliation réutilisait pour les événements déjà persistés un message d’erreur d’ingestion : `correct the named field at the signed event source, then redeliver; the rejected body was not journaled`. Une ligne invalide trouvée au replay est pourtant déjà dans l’inbox. Le conseil affirmait donc un état faux et suggérait une redelivery sans expliquer que l’append-only journal ne doit pas être édité.

Le test unitaire a été renforcé avant le changement de production. Un premier filtre Cargo fourni sans le chemin de module exact a lancé zéro test; il n’est pas compté. `--list` a donné `tests::invalid_event_payload_field_reports_persisted_journal_recovery_action`; l’invocation qualifiée a ensuite échoué comme prévu (0 réussi, 1 échoué) sur l’ancienne phrase. Les deux logs sont conservés, avec cette distinction.

## Changement et parcours processus

`reconciliation_error_value` expose maintenant `details.persistence_state="already_persisted"`. L’action demande d’inspecter le champ du payload signé d’origine, de préserver le journal, de ne pas traiter l’événement comme réconcilié et d’escalader vers une récupération fiable avant tout replay. Elle ne prétend plus que la livraison n’est pas journalisée.

Le scénario processus crée d’abord une review valide par le vrai receiver local, puis transforme dans son répertoire temporaire la ligne reçue en fixture hash-chain-valid dont le payload ne contient plus `action`. Le test lance le vrai binaire `forgejo-reconcile-local` avec son environnement vidé et réserve `127.0.0.1:3130` en écoute non bloquante. La CLI renvoie un résultat JSON `inconclusive/invalid_envelope`, `details.field_path=payload.action`, `persistence_state=already_persisted`, `upstream_read_attempts=0` et sans `canonical_state`. Aucun GET n’atteint le listener; le fichier de journal reste octet pour octet inchangé.

Ce cas ne valide pas le HMAC modifié : le scénario sert à exercer la validation du contenu **après** persistance, avec chaîne de hash de journal cohérente. La CLI n’a pas le secret de réception et ne peut pas authentifier à nouveau le corps. Il ne s’agit pas d’un webhook historique observé ni émis par Forgejo.

## Vérifications

- Rouge attendu : le test unitaire exact échoue sur l’ancienne guidance.
- Vert : `cargo test --offline --locked --bin forgejo-reconcile-local tests::invalid_event_payload_field_reports_persisted_journal_recovery_action -- --exact --nocapture` : 1/1.
- Processus isolé : `python3 scripts/run_webhook_reconcile_cli_e2e.py --scenario actual_reconcile_cli_process_refuses_persisted_review_without_action_before_api_read` : 1/1, 112 filtrés.
- `cargo fmt --all -- --check`, `git diff --check` et AST Python du runner réussissent.
- Le garde Cargo et le readback `cargo metadata --offline --locked --no-deps` confirment `license=null`, `license_file=null`, `publish=[]`, `decision=no_license_selected`, `redistribution_rights=not_assessed`, `spdx_package_license_declared=NOASSERTION`.

## Portée et limites

Les erreurs sont maintenant honnêtes et structurées, mais **aucun mécanisme de quarantaine, réparation, suppression logique ou réimport fiable n’est livré**. Le message escalade vers cette voie qui doit encore être conçue; ce lot ne débloque pas un événement ancien ni ne justifie une fusion. Test processus synthétique `n=1`, APIs en namespace réseau isolé; aucun Forgejo live, runner, PR, LLM/AgentOS, coût modèle ou mesure de productivité. La mission reste incomplète.

Les commandes, rouges/verts, versions, metadata, empreintes et vérification append-only des manifestes sont dans ce dossier. Les préfixes locaux, worktree et projet sont conservés.
