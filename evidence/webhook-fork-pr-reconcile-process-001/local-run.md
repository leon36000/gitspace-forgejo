# WEBHOOK-FORK-PR-RECONCILE-PROCESS-001

Date : 25 octobre 2026. Worktree observé au HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`, branche `local/WEBHOOK-005-canonical-links`, fortement modifié.

## Résultat

Le nouveau scénario `actual_reconcile_cli_process_reads_fork_pull_request_from_configured_target_api` compose l’entrée webhook réelle, l’inbox persistante et le vrai CLI de réconciliation pour un PR de fork.

1. Le test signe et envoie au vrai `forgejo-webhook-local` un événement `pull_request/synchronize`. `repository.full_name` et `pull_request.base.repo.full_name` sont `forge-admin/agent-forge-core`; `pull_request.head.repo.full_name` est `contributor/agent-forge-core`. Le receiver répond HTTP 202, `accepted=true`, `duplicate=false`, `reconcile_required=true`, puis s’arrête.
2. La fixture API loopback renvoie l’instantané courant du PR #43, avec base cible et tête de fork. Ses SHA sont différents de ceux du webhook.
3. Le vrai `forgejo-reconcile-local` lit l’inbox sans la modifier et demande une seule fois `/api/v1/repos/forge-admin/agent-forge-core/pulls/43`. Il n’envoie aucun header `Authorization`.
4. La sortie est `current_state_read` sur le PR #43. `canonical_state.head_sha` et `base_sha` sont les SHA actuels renvoyés par l’API fixture, pas la tête/base périmées du webhook; l’inbox reste identique octet pour octet.

Le scénario passe 1/1 sous Bubblewrap 0.9.0 avec `--unshare-net` et `--unshare-pid`; le test binaire compte 69 tests filtrés. Le runner construit depuis le code Cargo et le registre verrouillé hors réseau, puis exécute les vrais processus du receiver et du CLI. La preuve brute est `process-e2e.log`. Le contrat unitaire de base cible + tête de fork passe 1/1. Rustfmt, compilation Python du runner, `git diff --check` et le garde de métadonnées Cargo passent; le garde rapporte 7 tests `OK`.

## Vérification et tentatives

Le premier `rustfmt --check` a signalé un écart de mise en forme sur l’assertion ajoutée; `rustfmt` l’a corrigé avant la compilation et le run. Les logs conservés attestent les commandes finales. Aucun défaut de runtime n’a été observé et aucun code de production n’a été modifié dans cette tranche.

Le dernier `cargo test --offline --locked --all-targets -- --test-threads=1` documenté avant cet ajout test-only a passé 532 tests, 0 échec, 68 ignorés dans 34 groupes; ce run large n’a pas été répété. Le test de processus est explicitement `#[ignore]` et a été exécuté par son runner dédié, ce qui est la preuve pertinente pour son nouveau comportement.

## Sources et reproductibilité

- Contrat de la réponse courante : `tests/webhook_reconcile_contract.rs::pull_request_current_base_repository_is_bound_while_foreign_head_is_allowed`.
- Nouveau test processus : `tests/webhook_receiver_process_e2e.rs::actual_reconcile_cli_process_reads_fork_pull_request_from_configured_target_api`.
- Runner spécialisé : `scripts/run_webhook_reconcile_cli_e2e.py`.
- Commandes exactes : `commands.txt`.
- Versions : `toolchain.txt`.
- Empreintes des sources et du lockfile : `source-sha256.txt`.
- Toutes les sorties/commandes et le manifeste local : ce dossier.

## Portée et limites

C’est une composition locale de vrais processus Rust contre un serveur HTTP loopback de fixture, pas un événement émis par Forgejo vivant. Le fork et les réponses API sont synthétiques; aucun push Git depuis un dépôt fork, contrôle ACL Forgejo, PR live, autorité de fusion ou fusion n’est exercé. Le test établit que la notification fork est acceptée et que le CLI lit l’état courant au chemin du dépôt cible; il ne prouve pas l’autorisation de fusion ni le workflow complet avec agent.

Aucun LLM, AgentOS, Forge Code, RAG, moteur de mémoire, credential, service externe, dépense ou donnée privée n’a été utilisé. Le parcours LLM avec Opus attesté ailleurs est distinct; cela ne constitue pas une intégration modèle. Le crate reste `license=null`, `license_file=null`, `publish=[]`, décision `no_license_selected`, droits de redistribution `not_assessed`; aucune licence ni permission n’est inférée. Le projet reste incomplet et aucune comparaison de valeur n’est établie.

Suite utile, sans répéter ce run positif : ajouter le refus processus lorsque la réponse HTTP 200 annonce un `base.repo` étranger alors que la PR, son URL et ses SHA sont autrement valides; prouver `inconclusive/resource_mismatch`, aucun `canonical_state`, un seul GET du dépôt configuré et journal durable inchangé. Le contrat unitaire de ce mismatch existe déjà; l’écart restant est sa composition au vrai CLI.
