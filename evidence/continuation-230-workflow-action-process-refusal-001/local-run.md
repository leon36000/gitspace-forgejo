# CONTINUATION-230 — refus des couples workflow impossibles par le receiver réel

Date : 2026-10-27. Worktree local `webhook-005-workspace`, HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`, fortement modifié.

## Résultat

Le test du processus `forgejo-webhook-local` vérifie maintenant quatre refus : deux `action` inconnues et deux incompatibilités connues (`workflow_run_blocked` + `run_completed`, `workflow_job_waiting` + `job_completed`). Chaque livraison signée reçoit HTTP 400, code `event_payload_invalid`, `retryable=false` et `details.field_path=payload.action`; l’action conseille de consulter le contrat d’événement. Le test compare les octets de l’inbox après chaque refus et vérifie qu’aucune livraison n’a été journalisée.

Commande : `cargo test --locked --offline --test webhook_receiver_process_e2e actual_receiver_refuses_unknown_or_incompatible_workflow_actions_without_journaling -- --exact --nocapture` — 1/1 réussi. Format et `git diff --check` passent. Le log du processus et les sorties des garde-fous sont conservés dans ce dossier.

## État vérifié des interfaces agent et de la licence

Relecture du source courant : `src/agent_runtime.rs:4635-4644` définit un trait `AgentRuntimeV1`; le seul `impl AgentRuntimeV1` trouvé dans `src`, `tests` et `scripts` est `FixtureRuntime` sous `tests/agent_runtime_contract.rs:51`. Le README décrit ce fixture comme déterministe, sans LLM; il ne constate aucune intégration AgentOS/Hermes ou d’adaptateur de production. Aucun client LLM n’a été appelé et aucune intégration n’est ajoutée.

Le readback Cargo hors réseau donne `license=null`, `license_file=null`, `publish=[]`. Le garde rapporte `no_license_selected`, redistribution `not_assessed`, marqueur personnalisé `NOASSERTION`; aucune licence n’est choisie.

## Limites

Ce test lance le receiver local et exerce l’ingestion HTTP; il ne lance pas Forgejo, ses Actions ou le CLI de replay pour un couple incompatible persistant. Le contrat unitaire CONT-229 couvre les 14 couples, mais ce lot n’élargit pas cette couverture à une émission Forgejo réelle. Les réponses HTTP ne sont pas une preuve de correction du code et n’accordent aucune autorité de fusion. Aucun LLM/AgentOS, coût/tokens mesurés ou benchmark comparatif.

Prochaine action : reprendre la matrice de refus courante et cibler une lacune distincte avec son processus réel quand il est disponible; la comparaison agent+CI conventionnelle reste impossible sans runtime et budget autorisés. Le projet demeure incomplet.
