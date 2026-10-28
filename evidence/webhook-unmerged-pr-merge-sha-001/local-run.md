# WEBHOOK-UNMERGED-PR-MERGE-SHA-PROCESS-001

Date : 25 octobre 2026. Worktree : branche `local/WEBHOOK-005-canonical-links`, base `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`, checkout déjà fortement modifié; les fichiers préexistants ont été préservés.

## Résultat

Le test `actual_reconcile_cli_process_refuses_unmerged_pull_request_with_merge_commit_sha` complète le chemin receiver → journal durable → adaptateur HTTP → CLI réel. Le vrai processus `forgejo-webhook-local` reçoit une livraison `pull_request` synthétique signée et l’ajoute à l’inbox. Une fixture HTTP loopback sert HTTP 200 pour la PR 45, mais affirme `merged=false` tout en donnant un `merge_commit_sha`.

Le processus `forgejo-reconcile-local` lit `/api/v1/repos/forge-admin/agent-forge-core/pulls/45`, puis sort avec code 1 et un JSON `inconclusive`, `invalid_current_state`, non réessayable, `details.field=merge_commit_sha`; aucun `canonical_state` n’est émis. La fixture reçoit exactement un GET sans `Authorization`. Les octets du journal durable sont identiques avant et après le readback invalide. Le scénario ciblé passe 1/1 sous Bubblewrap 0.9.0, avec réseau, PID et session isolés; 55 tests sont filtrés.

Le test ne change pas le runtime de production : il verrouille au niveau processus une branche déjà implémentée dans `src/webhook/reconcile.rs`, selon laquelle un SHA de fusion présent sur une PR non fusionnée invalide le snapshot. Le contrat reste fail-closed même si le transport répond 200.

## Vérifications

Commande : `python3 -B scripts/run_webhook_reconcile_cli_e2e.py --scenario actual_reconcile_cli_process_refuses_unmerged_pull_request_with_merge_commit_sha` (exit 0; 1 passed, 0 failed, 55 filtered).

`rustfmt --edition 2024 --check tests/webhook_receiver_process_e2e.rs`, `cargo fmt --all -- --check`, `git diff --check` sur les deux fichiers touchés, la syntaxe du runner Python et `scripts/verify_cargo_license_state.py` passent. Le readback Cargo reste `license=null`, `license_file=null`, `publish=[]`, `no_license_selected`, `redistribution_rights=not_assessed`; aucune licence n’est choisie.

Versions, commandes, empreintes et sortie complète sont dans les artefacts voisins. Le manifeste local passe 9/9.

## Limites

Le serveur API est une fixture HTTP loopback; aucune instance Forgejo vivante ou donnée privée n’est utilisée. Un seul scénario est exécuté; la suite `--all-targets` n’est pas rejouée. Ce cas ne valide pas l’existence réelle du commit annoncé par une réponse valide, son arbre, un modèle LLM/AgentOS, ni un gain comparatif. Aucun code Forgejo n’est copié; aucun appel externe, dépense, publication ou changement de production n’a lieu.

La mission reste incomplète. Le prochain lot poursuit le protocole scratch de publication/fencing LFS et les lacunes de réconciliation composées sans rejouer ces scénarios.
