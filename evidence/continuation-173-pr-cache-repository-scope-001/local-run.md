# CONTINUATION-173 — scope du dépôt pour un batch de PR homonymes

Date : 26 octobre 2026. Worktree : `/home/pc6/Documents/Codex/2026-10-21-mission-construire-une-forge-con-ue/.runtime/webhook-005-workspace`, base HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`; dépôt déjà fortement modifié, travaux antérieurs conservés. Cette tranche modifie le test processus et son allowlist dans le runner, plus les documents de suivi. Aucun code de production, dépendance, Cargo.toml ou lockfile n’a changé.

## Scénario

Le test `actual_reconcile_cli_process_refuses_unconfigured_same_number_pull_request_in_mixed_batch` envoie par le vrai receiver Rust deux événements `pull_request` signés avec la clé synthétique : PR #23 du dépôt configuré `forge-admin/agent-forge-core`, puis PR #23 de `forge-admin/agent-forge-sibling`. Le receiver accepte les deux en HTTP 202, séquences 1 et 2, sans doublon. Le CLI Rust est lancé sur les deux IDs.

Un premier essai (journal conservé dans [`process-test.log`](process-test.log)) attendait deux GET et fournissait un DTO pour chaque dépôt. Le fixture a expiré après une seule requête avec `reconciliation CLI made too few requests`. Ce rouge provenait d’une hypothèse erronée du test, pas d’un défaut détecté : le CLI est configuré pour un dépôt et refuse l’événement du dépôt frère avant d’émettre une route API. Le test a été recadré pour contrôler explicitement cette frontière, sans changement de comportement produit. Le reçu final source-courant est [`process-test-final-002.log`](process-test-final-002.log).

Commande finale :

```text
python3 scripts/run_webhook_reconcile_cli_e2e.py --scenario actual_reconcile_cli_process_refuses_unconfigured_same_number_pull_request_in_mixed_batch
```

Résultat ciblé source-courant : `1 passed; 0 failed; 0 ignored; 129 filtered out` dans [`process-test-final-003.log`](process-test-final-003.log). Sous Bubblewrap 0.9.0, le test exécute le vrai receiver, journal et CLI, avec namespace réseau/PID distinct, source staged read-only, environnement vidé et `/tmp` isolé. Il démarre seulement une API fixture sur `127.0.0.1:3130`.

## Assertions

- Les deux webhooks sont acceptés et le batch conserve l’ordre.
- Un seul GET vise `/repos/forge-admin/agent-forge-core/pulls/23`, sans en-tête `Authorization`.
- Le premier résultat est `current_state_read` avec le head/base/reviewer/équipe du DTO du dépôt configuré.
- Le second est `inconclusive/resource_out_of_scope`, `retryable=false`, lié aux noms configuré et event, sans `canonical_state` ni objet `details.resource`; l’action ordonne le routage vers l’adaptateur correspondant et dit de ne pas réessayer sur le client courant.
- Aucun GET ne vise le dépôt frère; le total des lectures amont vaut 1.
- Le journal durable reste identique octet pour octet.

Les hashes des sources sont dans [`source-hashes.txt`](source-hashes.txt). Les versions et l’isolation sont dans [`environment.txt`](environment.txt). Les checks finaux `cargo fmt --all -- --check`, `git diff --check`, `python3 -m py_compile scripts/run_webhook_reconcile_cli_e2e.py`, la garde de métadonnées et le readback Cargo réussissent; leurs sorties sont jointes.

Readback Cargo : `agent-forge-core 0.1.0`, `license=null`, `license_file=null`, `publish=[]`, `decision=no_license_selected`, `redistribution_rights=not_assessed`, champ personnalisé `spdx_package_license_declared=NOASSERTION`. Aucune licence ou permission de redistribution n’est choisie ou déduite.

## Portée

Cette preuve vérifie la frontière du CLI configuré pour un seul dépôt face à deux PR de même numéro. Elle ne prouve pas le routage vers plusieurs adaptateurs, la sécurité des ACL côté serveur, la clé de cache cross-repository d’un client multi-dépôts ou un serveur Forgejo live. Aucun LLM, AgentOS, Forge Code, credential de fusion, coût externe, service externe, production ou code upstream n’a été utilisé. Aucun test all-targets n’a été relancé. La mission globale reste ouverte.
