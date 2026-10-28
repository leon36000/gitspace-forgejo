# CONTINUATION-046 — observation HTTP jusqu’à la fin du CLI

Date : 26 octobre 2026. Worktree : `webhook-005-workspace`, HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. Les modifications préexistantes ont été conservées; aucun commit n’a été créé.

## Problème et changement

La fixture `start_scripted_cli_http_fixture` fermait son listener après le nombre de réponses attendu et une durée de silence fixe. Elle était utilisée par des tests processus qui concluaient qu’un seul GET avait eu lieu. Une requête envoyée après l’expiration de cette fenêtre échouait au niveau TCP sans apparaître dans le journal de la fixture.

Le test `scripted_http_fixture_stays_open_until_explicitly_stopped` a d’abord échoué avec le comportement antérieur : après la fenêtre de 50 ms, la fixture n’acceptait plus une deuxième requête envoyée 150 ms plus tard. La sortie du runner est `red.log`; son code de sortie attendu avant changement est conservé dans `red.exit-code` (1).

Le helper offre maintenant un mode jusqu’au signal d’arrêt. Le processus appelant envoie ce signal après la fin du CLI; la fixture accepte toutes les connexions reçues jusque-là, puis draine pendant 100 ms les connexions déjà en attente. Le mode à fenêtre de silence demeure disponible aux autres tests. Le premier essai de la nouvelle boucle a révélé qu’elle ne consultait pas le canal d’arrêt pendant l’attente de `accept`; le test dédié a échoué par délai de 10 secondes. La boucle a été corrigée pour reprendre le contrôle et consulter le signal après chaque poll non bloquant; les résultats verts ci-dessous sont postérieurs à cette correction.

Les tests `actual_reconcile_cli_process_reports_actionable_upstream_401_without_leaking_credential` et `actual_reconcile_cli_process_preserves_out_of_scope_refusal_after_401` utilisent le mode jusqu’au signal. Après la sortie du vrai binaire CLI, ils arrêtent la fixture et valident sa liste indépendante de requêtes : exactement un GET configuré avec le credential synthétique pour le batch de deux livraisons dupliquées; exactement un GET de l’issue configurée pour le batch mixte, aucun GET pour l’événement hors portée. Ils vérifient aussi la réponse 401/actionnable, l’absence de credential et de corps d’erreur dans stdout/stderr, et la conservation octet pour octet de l’inbox dans le scénario mixte.

## Vérifications

- Reproduction pré-changement : échec attendu parce que la deuxième connexion tardive ne peut pas joindre le listener fermé (`red.log`, code 1 dans `red.exit-code`).
- Test du helper après correction : 1 réussi, 0 échec; deuxième requête observée après 250 ms (`fixture-green.log`).
- Processus CLI, 401 terminal et doublons : 1 réussi, 0 échec (`cli-401-green.log`).
- Processus CLI, refus hors portée après 401 : 1 réussi, 0 échec (`out-of-scope-401-green.log`).
- Compatibilité du mode historique à fenêtre silencieuse avec refus 401 et reprise après rotation du credential : 1 réussi, 0 échec (`quiet-window-compat-green.log`).
- Target `webhook_receiver_process_e2e` : 28 réussis, 0 échec, 63 ignorés (`process-target.log`). Les scénarios ignorés n’ont pas été comptés comme réussis.
- Garde Cargo : 9 tests réussis. Le readback confirme `license=null`, `license_file=null`, `publish=[]`, `decision=no_license_selected`, `redistribution_rights=not_assessed`, `spdx_package_license_declared=NOASSERTION` (`cargo-license-readback.json`). Aucun choix de licence n’a été fait.
- `cargo fmt --all -- --check`, byte-compilation du runner et `git diff --check` passent (`static-verification.log`).

Outils : Rust/Cargo et Python indiqués dans `toolchain.txt`; les trois tests processus ciblés sont lancés par `scripts/run_webhook_reconcile_cli_e2e.py`, qui compile les sources sous Bubblewrap avec `--unshare-net --unshare-pid` et exécute les scénarios isolés. Les logs complets et empreintes de sources sont conservés dans ce dossier.

## Portée et limites

Ce lot améliore la preuve d’observation du banc et la liaison des requêtes HTTP capturées à la sortie du CLI; il ne change aucun code runtime de l’adaptateur, du webhook ou du gate. Le partenaire est une fixture loopback synthétique, pas Forgejo live. Le test de requête tardive est synthétique; il démontre le comportement du listener de test et non un défaut du CLI en production. Les assertions exactes renforcées ne couvrent que les deux parcours 401 migrés; les autres cas conservent leur mode d’observation existant. Pas d’AgentOS/LLM, d’appel externe, de secret réel, de coût, de publication ou de production.

Le projet reste incomplet. Ce résultat ne démontre ni le parcours LLM demandé au départ ni un gain comparatif; il ne résout pas les limites CAS Forgejo contre les écrivains externes.
