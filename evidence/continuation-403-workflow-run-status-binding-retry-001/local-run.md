# CONTINUATION-403 — statuts `workflow_run` / `workflow_job` liés à l’en-tête signé

Date : 28 octobre 2026. Worktree `/home/pc6/Documents/Codex/2026-10-21-mission-construire-une-forge-con-ue/.runtime/webhook-005-workspace`, branche `local/WEBHOOK-005-canonical-links`, HEAD de base `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. Le checkout était déjà très modifié; aucun état antérieur n’a été réinitialisé, écrasé, commité ou publié.

## Défaut et correction

La validation rejetait déjà une discordance entre le statut signé (`run.status` ou `job.status`) et le statut attendu par `X-Forgejo-Event`, mais le chemin `workflow_run` la classait comme `ResourceMismatch`. Le receiver renvoyait donc une erreur sans `details.field_path`. La première exécution processus échoue sur `field_path=null`, attendait `payload.run.status` : [red-missing-status-path.log](red-missing-status-path.log). La reprise immédiate reproduit le même défaut, ce qui a révélé que la branche source retournait encore `ResourceMismatch` : [isolated-green.log](isolated-green.log).

La discordance est maintenant `InvalidEventPayloadField("run.status")`; les jobs utilisent leur champ publié `job.status` au lieu de renvoyer à tort `run.status`. Le mapping HTTP expose le chemin précis et compare le corps signé à l’en-tête non signé. Il demande de faire corriger l’en-tête par le sender/forwarder de confiance, puis de redélivrer le corps signé inchangé sous le même delivery ID. Il déconseille explicitement de modifier le corps signé. Les contrats et tests reflètent les deux chemins.

Le test processus `actual_receiver_refuses_workflow_job_status_header_substitution_before_append` vérifie HTTP 400, `payload.job.status`, ce conseil, l’inbox vide, puis l’admission du même corps sous l’en-tête correct en séquence 1. Une première exécution a révélé que la fixture positive omettait `job.html_url`; l’URL requise a été ajoutée à la fixture, et la preuve est [status-binding-bwrap-post-fixture.log](status-binding-bwrap-post-fixture.log). Une assertion test-first a aussi reproduit l’ancien conseil dangereux/incomplet avant le nouveau texte : [red-signed-body-guidance.log](red-signed-body-guidance.log).

Le scénario `actual_reconcile_cli_process_reads_workflow_run_canonically_and_projects_safe_fields` vérifie en un flux que `run.id=0` est refusé sans journalisation, puis que le statut signé `success` sous l’en-tête incompatible `workflow_run_failure` est refusé avec `payload.run.status`. Le retry garde le corps signé et ne corrige que l’en-tête; il est admis à la séquence 1 sous le même delivery ID. Le vrai CLI effectue ensuite un GET API de fixture, observe l’état courant `failure` plutôt que l’ancien `success`, n’émet pas les marqueurs privés et ne modifie pas le journal. L’attestation vérifiée est locale et utilise une clé Ed25519 synthétique explicitement approuvée pour le test. Les deux scénarios isolés passent : [status-binding-bwrap-final.log](status-binding-bwrap-final.log).

## Vérification

```text
python3 scripts/run_webhook_reconcile_cli_e2e.py \
  --scenario actual_receiver_refuses_workflow_job_status_header_substitution_before_append \
  --scenario actual_reconcile_cli_process_reads_workflow_run_canonically_and_projects_safe_fields
2 passés, 0 échec; Bubblewrap --unshare-net --unshare-pid

cargo test --offline --locked --lib workflow_action_and_status_contracts_are_published_and_drive_payload_validation
1 passé, 0 échec

cargo test --offline --locked --test webhook_reconcile_contract workflow_run_success_webhook_uses_current_api_state_and_redacts_event_payload
1 passé, 0 échec

cargo test --offline --locked --all-targets -- --test-threads=1
755 passés, 0 échec, 188 ignorés; 38 groupes

cargo fmt --all -- --check
git diff --check
python3 scripts/verify_cargo_license_state.py
toutes ces commandes terminent avec le code 0
```

Les deux tests processus ciblés sont marqués `ignore` dans Cargo et ont été exécutés séparément dans la sandbox. Les autres tests ignorés, dont 186 dans cette cible, ne sont pas prouvés par cette exécution. Les sorties intermédiaires de diagnostic sont conservées dans ce dossier; les logs finaux sont `status-binding-bwrap-final.log`, `cargo-all-targets-final.log`, `rustfmt-final.log`, `diff-check-final.log` et `cargo-license-final.json`. Outils et empreintes des sources : `tool-versions.txt` et `source-hashes.txt`.

La garde Cargo reste `license=null`, `license_file=null`, `publish=[]`, `no_license_selected`, droits de redistribution `not_assessed` et SPDX `NOASSERTION`. Aucun code tiers, modèle, credential réel ou dépendance n’a été ajouté.

## Portée et limites

Cela prouve le refus des statuts/header discordants et la correction sûre au niveau du vrai processus receiver local, ainsi que le readback du CLI vers une API loopback fixture. Cela ne prouve ni l’émission d’un webhook natif Forgejo, ni un Forgejo live, ni AgentOS/LLM, PR/fusion, sécurité du sender, comportement de production, métrique de réussite ou gain comparatif. L’attestation atteste la signature de fixture, pas la vérité du contenu. Le projet complet reste non démontré.

Suite locale utile : mesurer au niveau de l’OS que le processus agent isolé ne peut atteindre les routes Forgejo API/Web/receive-pack ou le credential de fusion, tandis que le vérificateur/gate ne reçoit que les capacités réseau prévues; distinguer une allowlist client d’une frontière processus/réseau réellement imposée.
