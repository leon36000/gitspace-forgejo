# CONTINUATION-009 — découverte de la frontière credential hôte/agent

Date : 25 octobre 2026. Dépôt local : `agent-forge-core`. HEAD de référence : `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`; le worktree était déjà fortement modifié et reste non commité. Cette preuve porte sur le changement de contrat de capacités v1 vers v2 dans le worktree courant, pas sur un commit propre isolé.

## Question

Le CLI `forgejo-reconcile-local --host-credential-fd N` garantissait-il que le token n’était pas accessible au processus de l’agent, ou seulement qu’il ne figurait pas dans certains canaux observables ?

## Constat de code

L’examen ciblé a suivi `read_private_credential_pipe` → `reconciliation_api` / `api_with_host_token` → `LocalLoopbackForgejoApi::request_api_once` → `build_curl_api_request` → `run_command_bounded_with_input`. Le CLI lit les octets dans son espace mémoire; la configuration `Authorization` est passée au processus curl dans stdin (`--config -`) et curl détient le token pendant la requête. Les tests de contrat précédents excluent argv, environnement, journal et réponse, mais le binaire ne crée ni ne vérifie une séparation OS entre l’agent et son propre processus. Il était donc incorrect de résumer cette propriété comme « credential hors de l’agent ».

## Changement

`capabilities_document` annonce maintenant `forgejo-reconcile-capabilities-v2`, version de schéma 2, et un objet `credential_boundary` qui déclare explicitement :

- les emplacements mémoire du token : processus adaptateur et processus enfant curl;
- `agent_process_isolation: not_enforced_by_cli`;
- la responsabilité requise de l’appelant : exécuter ce chemin avec credential depuis un processus hôte de confiance hors du périmètre d’exécution agent.

Les opérations de réconciliation et d’autorité de fusion référencent ce contrat commun. Un commentaire dans `read_private_credential_pipe` indique aussi les emplacements réels et la limite. Le nouveau contrat ne met pas lui-même en place une isolation; un lanceur qui ne la garantit pas ne doit pas fournir de credential à un agent.

## Vérification

Le test processus a d’abord échoué sur le document v1 (`schema_version` observée 1, attendue 2); voir [capabilities-v2-red.log](capabilities-v2-red.log). Après correction :

- `cargo test --offline --locked --bin forgejo-reconcile-local capabilities_list_exact_merge_target_profiles -- --nocapture` : 1 réussi, 0 échec;
- `cargo test --offline --locked --test forgejo_reconcile_capabilities_e2e actual_cli_discloses_versioned_local_test_capabilities -- --nocapture` : 1 réussi, 0 échec;
- `cargo build --offline --locked --bin forgejo-reconcile-local` : réussi;
- `target/debug/forgejo-reconcile-local --capabilities` : readback réel du binaire, résumé dans [readback-summary.json](readback-summary.json);
- `cargo fmt --all -- --check`, `git diff --check` et `python3 scripts/verify_cargo_license_state.py` : réussis.

Rust/Cargo : voir les versions en fin de ce rapport. La sortie JSON complète, les logs et les sommes de contrôle sont dans ce dossier.

Le manifeste local du dossier couvre les 14 autres fichiers de preuve et passe `sha256sum -c SHA256SUMS`.

## Portée et réserves

Cette tranche vérifie une découverte versionnée du contrat et le comportement du processus CLI qui la rend. Elle ne teste pas l’isolation OS, l’identité de l’émetteur du pipe, les permissions Forgejo attachées au token, la séparation multi-utilisateur ou le comportement d’un modèle. Aucun LLM, AgentOS, Forgejo live, credential réel, service externe ou coût n’a été utilisé. La lecture live précédente de PR privée est référencée par CONTINUATION-008 et n’a pas été répétée.

Le changement ne complète pas le parcours LLM ni la comparaison appariée Forgejo + agent + CI; il ne justifie aucun gain général. Avant une intégration authentifiée, l’appelant doit appliquer la séparation de processus déclarée. HEAD reste inchangé; aucun commit, fork, publication ou changement de production n’a été fait.

## Sources locales consultées

- `src/bin/forgejo-reconcile-local.rs`: `read_private_credential_pipe`, `LocalLoopbackForgejoApi::request_api_once`, `build_curl_api_request`, `capabilities_document`, `tests::capabilities_list_exact_merge_target_profiles`.
- `tests/forgejo_reconcile_capabilities_e2e.rs`: `actual_cli_discloses_versioned_local_test_capabilities`.
- `docs/checkpoint.md`, `docs/architecture.md`, `docs/implementation-plan.md`.
- Versions de l’outil Rust/Cargo : [tool-versions.txt](tool-versions.txt).
