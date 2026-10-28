# CONTINUATION-402 — retry `workflow_run` corrigé jusqu’au readback canonique

Date : 28 octobre 2026. Worktree `/home/pc6/Documents/Codex/2026-10-21-mission-construire-une-forge-con-ue/.runtime/webhook-005-workspace`, branche `local/WEBHOOK-005-canonical-links`, HEAD de base `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. Le checkout demeure fortement modifié; aucun changement antérieur n’a été réinitialisé, écrasé, stage ou commité.

## Défaut reproduit et correction

Le validateur de `workflow_run` refuse déjà `run.id=0` comme `InvalidEventPayloadField("run.id")`, mais le receiver renvoyait encore le conseil générique. `src/webhook/http.rs` indique maintenant de fournir `payload.run.id` comme entier JSON positif, de consulter `GET /forgejo/capabilities` puis de redélivrer sous le même delivery ID; le corps refusé n’est pas journalisé. Le validateur et les permissions restent inchangés.

Le test existant `actual_reconcile_cli_process_reads_workflow_run_canonically_and_projects_safe_fields` a été renforcé. Il signe l’événement `workflow_run_success` avec `run.id=0`, vérifie HTTP 400, `retryable=false`, `field_path=payload.run.id`, le conseil et l’absence d’écriture. Le même ID est ensuite renvoyé avec `run.id=7001`; le receiver l’accepte à la séquence 1 et l’atteste avec une clé Ed25519 synthétique. Le vrai CLI de réconciliation reçoit explicitement la clé publique de fixture et lit une seule fois `/api/v1/repos/forge-admin/agent-forge-core/actions/runs/7001`. L’événement dit `success`, tandis que la réponse API courante dit `failure`; la sortie canonique reflète `failure`, le SHA et l’horodatage courants. Les marqueurs privés des deux corps n’apparaissent pas dans la sortie et le journal reste octet-identique.

Passe rouge : le runner Bubblewrap exécute le scénario exact avant le mapping et échoue sur le texte générique. Après le mapping, une première passe atteint l’assertion d’attestation et expose une configuration de test incomplète : le test lançait un receiver non signant, tout en exigeant `verified`. Le test est corrigé pour utiliser `start_ready_attested_receiver` et fournir au CLI la clé publique de confiance; l’assertion d’attestation vérifiée est conservée. Le scénario corrigé passe ensuite 1/1. Le premier échec de formatage était un simple retour à la ligne; il a été corrigé et `rustfmt --check` repasse. Tous ces journaux intermédiaires sont conservés.

## Vérification finale

```text
python3 scripts/run_webhook_reconcile_cli_e2e.py --scenario actual_reconcile_cli_process_reads_workflow_run_canonically_and_projects_safe_fields
1 passed; 0 failed; sous Bubblewrap --unshare-net --unshare-pid

cargo test --offline --locked --all-targets -- --test-threads=1
755 passed; 0 failed; 188 ignored; 38 groupes

cargo fmt --all -- --check
exit 0

git diff --check
exit 0

python3 scripts/verify_cargo_license_state.py
metadata_policy_verified
```

Le scénario ciblé est ignoré par défaut. All-targets marque 188 tests ignorés; ce scénario en fait partie et a été exécuté séparément par le runner d’isolation. Les autres tests ignorés n’ont pas été exercés dans ce lot. La sandbox monte les sources sélectionnées en lecture seule, utilise un réseau et un espace PID privés, un cache Cargo pré-stage hors ligne et une API fixture loopback sur le port attendu. Outils et empreintes des sources finales : `tool-versions.txt` et `source-hashes.txt`. Reçus : `red-generic-guidance.log`, `isolated-green.log` (configuration d’attestation d’abord incomplète), `isolated-green-final.log`, `isolated-green-post-format.log`, `cargo-all-targets-post-format.log`, `rustfmt-final.log` et `rustfmt-final-pass.log`, `diff-check-final.log`, `license-state-final.json`.

Cargo conserve `license=null`, `license_file=null`, `publish=[]`, `no_license_selected`, droits de redistribution `not_assessed` et SPDX `NOASSERTION`. Aucun choix de licence ou dépendance n’est ajouté.

## Portée et limites

Ce parcours prouve l’intégration receiver → inbox durable → CLI → API fixture pour un retry corrigé et l’usage de l’état courant. L’API est synthétique : ce n’est pas un Forgejo vivant, un webhook natif, une PR ou une fusion. L’attestation vérifiée porte uniquement sur la signature du receiver local sous la clé de confiance de fixture; elle n’authentifie pas la vérité amont, le rédacteur local ou la correction du logiciel. Aucun AgentOS/LLM, donnée privée, service externe ou credential réel n’est utilisé. Le projet reste incomplet.
