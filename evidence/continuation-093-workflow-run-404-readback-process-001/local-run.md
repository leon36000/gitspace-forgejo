# CONTINUATION-093 — traiter un `workflow_run` disparu/inaccessible comme inconclusif

Date : 26 octobre 2026. Worktree produit : `/home/pc6/Documents/Codex/2026-10-21-mission-construire-une-forge-con-ue/.runtime/webhook-005-workspace`; HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`, worktree fortement modifié.

## Scénario et résultat

Le vrai receiver accepte HTTP 202 pour une notification HMAC `workflow_run_success` valide, liée au dépôt configuré, run 7001/index 41. Le readback de l’API fixture renvoie HTTP 404 et un corps contenant `workflow-run-404-private-marker`. Le vrai CLI `forgejo-reconcile-local`, exécuté sous Bubblewrap, fait exactement un GET sur le run attendu puis émet un JSON `inconclusive/resource_not_found`, `retryable=false`, avec le type `workflow_run`, les identifiants attendus et une action de diagnostic. Aucun `canonical_state` ni marqueur du webhook/404 n’est divulgué; la requête n’envoie aucun Authorization et le journal accepté reste identique octet pour octet.

Test : `actual_reconcile_cli_process_keeps_workflow_run_404_inconclusive_without_projection`. Commande :

```text
python3 scripts/run_webhook_reconcile_cli_e2e.py --scenario actual_reconcile_cli_process_keeps_workflow_run_404_inconclusive_without_projection
```

Le driver compile hors ligne puis lance le test processus sous Bubblewrap avec namespaces réseau/PID isolés, environnement effacé et sources read-only. Résultat ciblé : 1 passé, 0 échec, 105 filtrés.

## Vérifications

- `cargo test --offline --locked --all-targets -- --test-threads=1 --quiet` : 34 exécutables, 597 réussis, 0 échec, 98 ignorés.
- `cargo fmt --all -- --check`, `git diff --check` et `python3 -m py_compile scripts/run_webhook_reconcile_cli_e2e.py` passent.
- `python3 -m unittest scripts.test_cargo_license_state -v` : 9/9.
- `python3 scripts/verify_cargo_license_state.py` confirme `license=null`, `license_file=null`, `publish=[]`, `license_decision=no_license_selected`, `redistribution_rights=not_assessed`, `spdx_package_license_declared=NOASSERTION`, `status=verified`.
- Versions et SHA-256 des sources : `tool-versions.txt`, `source-sha256.txt`; les sorties de vérification sont conservées dans ce dossier.

## Portée et limites

Le processus réel et le code de réconciliation courant sont testés, mais le serveur amont est une fixture loopback, pas Forgejo live ni un runner Actions. HTTP 404 ne permet pas de distinguer un run supprimé/absent d’un dépôt privé inaccessible; le résultat est explicitement inconclusif et n’autorise aucune fusion. `n=1`; pas de résultat de valeur ou de fiabilité générale. Aucun code runtime, dépendance, lockfile ou configuration de production n’a changé. Aucun LLM/AgentOS, coût ou service externe.

Le champ Cargo reste sans licence choisie et la publication désactivée; cela résout les métadonnées sans accorder de droits. GitSpace et les droits de redistribution restent non établis. La mission demeure incomplète, en particulier pour la comparaison Forgejo+agent+CI avec coûts/tokens contrôlés et pour un runtime agent local réellement autorisé.
