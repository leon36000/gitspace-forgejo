# WEBHOOK-SAME-ORIGIN-FOREIGN-REPOSITORY-PROCESS-001

Date : 25 octobre 2026. Worktree `local/WEBHOOK-005-canonical-links`, base de travail HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`.

## Résultat

Le nouveau test `actual_reconcile_cli_process_rejects_same_origin_foreign_repository_html_url` traverse le vrai binaire `forgejo-webhook-local`, son inbox durable et le vrai `forgejo-reconcile-local`. Le receiver accepte une livraison d’issue synthétique signée pour `forge-admin/agent-forge-core`. Une fixture API loopback répond ensuite HTTP 200 avec le bon numéro, état et horodatage, mais un `html_url` sur la même origine (`127.0.0.1:3130`) qui désigne `other-owner/other-repository`.

Le CLI termine en refus structuré `inconclusive/resource_mismatch`, non réessayable, sans `canonical_state`. Il a effectué un seul GET sur le chemin du dépôt configuré, sans en-tête Authorization. L’inbox est identique octet par octet après lecture. Ce test complète le contrat unitaire de mismatch de dépôt et le test processus distinct qui refusait une origine étrangère; il ne modifie pas le comportement de production.

## Vérifications

- Runner isolé : 1 test passé, 0 échec, 67 filtrés; la compilation Cargo est `--offline --locked`.
- Bubblewrap désolidarise les espaces réseau et PID; la source est montée en lecture seule dans l’environnement, et les fixtures API sont loopback.
- `rustfmt --edition 2024 --check tests/webhook_receiver_process_e2e.rs`, compilation Python du runner et `git diff --check` passent.
- Garde de non-sélection Cargo : 7 tests passent. Le readback frais donne `license=null`, `license_file=null`, `publish=[]`, `license_decision=no_license_selected`, `redistribution_rights=not_assessed`.

Les commandes, sorties, versions et hashes sont dans ce dossier.

## Limites

L’API est une fixture HTTP locale, pas une instance Forgejo. Le test couvre une issue et un seul processus/résultat (`n=1`); il ne prouve ni le comportement contre un serveur hostile, ni une séparation entre UID, ni un parcours LLM/AgentOS, ni un gain comparatif. Aucun credential réel, coût, service externe ou donnée privée n’a été utilisé. La licence produit demeure non choisie; aucune autorisation de redistribution n’est inférée.
