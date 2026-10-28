# CONTINUATION-244 — retry récupéré partagé dans un batch; métadonnées Cargo vérifiées

Date : 27 octobre 2026. Worktree préexistant `.runtime/webhook-005-workspace`; HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. Les fichiers partaient d’un état déjà très modifié; aucun reset, commit, publication, service de production ou appel externe.

## Tranche adapter/webhook

Le test ajouté `actual_reconcile_cli_process_shares_recovered_issue_read_across_batch_deliveries` suit deux UUID distincts portant le même numéro d’issue. Le receiver Rust accepte et journalise les deux webhooks HMAC. Le CLI Rust réel traite les deux IDs dans le même batch; sa fixture HTTP locale renvoie `429 Retry-After: 0`, puis un état d’issue `open` en HTTP 200. Les deux résultats sont `current_state_read`, avec leurs IDs distincts et le même readback courant. Exactement deux requêtes sont observées (429 + 200), donc le readback récupéré n’est pas relu pour le second événement.

Le test compare les octets du journal avant/après, vérifie la route GET attendue, l’absence de `Authorization`, l’absence du corps marqueur 429 dans la sortie et stderr vide. Exécution finale via `python3 scripts/run_webhook_reconcile_cli_e2e.py --scenario actual_reconcile_cli_process_shares_recovered_issue_read_across_batch_deliveries`: 1 réussi, 149 filtrés, sous Bubblewrap `--unshare-net --unshare-pid`; la cible Rust et les binaires nécessaires sont compilés offline dans le wrapper.

Le changement est limité au test d’intégration et à l’allowlist du runner; aucun code du runtime Rust n’a été édité. Le scénario prouve une règle de cache intra-batch sur une fixture locale; il ne prouve pas une réponse d’API Forgejo, un snapshot atomique, un délai positif ni un gain général.

## Métadonnées de licence Cargo

Readback frais de `cargo metadata --offline --locked --format-version 1 --no-deps` et garde `scripts/verify_cargo_license_state.py`: paquet `agent-forge-core`, `license=null`, `license_file=null`, `publish=[]`, `decision=no_license_selected`, `redistribution_rights=not_assessed`, `spdx_package_license_declared=NOASSERTION`. Les neuf tests de la garde passent. Aucune licence ni texte de licence n’a été sélectionné ou créé; la publication Cargo reste désactivée. Ce contrôle exprime l’indécision et ne décide pas les droits de redistribution de Forgejo, GitSpace ou des dépendances.

## Vérifications et limites

`cargo fmt --all -- --check`, `git diff --check` et l’AST Python du runner passent. La suite générale Rust n’a pas été relancée; les sorties des deux suites ciblées et des commandes sont archivées ici. Pas de Forgejo live, LLM/AgentOS, nouvelle fusion ou comparaison contrôlée. Le projet reste incomplet.
