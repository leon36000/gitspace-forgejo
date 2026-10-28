# CONTINUATION-287 — correction d’action webhook, retry idempotent et état API courant

Date : 27 octobre 2026. Worktree local fortement modifié, HEAD de base `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. Aucun commit, publication, coût, appel modèle, service externe ou changement de production.

## Changement

Pour un refus `event_payload_invalid` sur `payload.action`, le receiver indique maintenant de lire les actions prises en charge dans `GET /forgejo/capabilities`, de corriger le payload et de redélivrer le même delivery ID. Le corps rejeté n’a pas été journalisé. `retryable=false` reste inchangé : le message distingue une correction humaine d’un retry automatique qui renverrait les mêmes octets. Les autres champs invalides conservent leur action générique.

Le test composé `rejected_issue_action_can_retry_same_delivery_then_reconciles_current_api_state` vérifie le chemin handler→journal→réconciliation : un événement `issues` HMAC signé sans action reçoit HTTP 400 et ne réserve pas l’ID; un événement `opened` corrigé réutilise cet ID et obtient séquence 1; un doublon est accepté comme duplicate sans append; la réconciliation fait exactement une lecture de l’issue courante. Le payload webhook dit `open`, l’API fixture dit `closed`, et seule la valeur `closed` est projetée.

TDD : avant le changement HTTP, le nouveau test échoue précisément parce que `error.action` ne mentionne pas la redélivrance avec le même ID. Après la correction, le test ciblé passe; voir `tdd-red-observation.txt` et `tdd-green-targeted.log`.

## Vérification

- `cargo test --locked --offline --test webhook_reconcile_pipeline` : **5/5**, 0 échec.
- `cargo test --locked --offline --all-targets` : **694 réussis, 0 échec, 151 ignorés, 37 suites**. Les tests ignorés restent hors couverture de cette commande.
- `cargo fmt --all -- --check` et `git diff --check` : code 0.
- `cargo metadata --offline --locked --no-deps --format-version=1` et `scripts/verify_cargo_license_state.py` : package `agent-forge-core`, version `0.1.0`, `license=null`, `license_file=null`, `publish=[]`, `no_license_selected`, `redistribution_rights=not_assessed`, `NOASSERTION`.
- `python3 -m unittest discover -s scripts -p 'test_cargo_license_state.py'` : **12/12**.

La déclaration Cargo est donc sans licence choisie et sans publication autorisée. Elle n’accorde aucun droit de redistribution et ne constitue pas l’inventaire légal complet du produit ou de ses dépendances.

## Portée et limites

Le test est une suite d’intégration Rust qui utilise le vrai `handle_webhook_request`, le vrai journal `EventJournal` et le vrai `reconcile_inbox_events`, avec une API Forgejo simulée par fixture. Il ne lance pas le binaire receiver comme processus et ne contacte pas Forgejo live; ce prochain niveau de preuve reste ouvert. Le lot ne teste ni AgentOS/LLM ni la réaction d’un modèle au message; aucun benchmark ou gain n’est démontré. Un run déterministe vérifie un contrat précis, pas le comportement de tous les clients Forgejo.

## Artefacts

`commands.txt`, `tdd-red-observation.txt`, `tdd-green-targeted.log`, `webhook-reconcile-pipeline-tests.log`, `cargo-all-targets.log`, `cargo-fmt.log`, `git-diff-check.log`, `cargo-metadata.json`, `cargo-license-state.json`, `cargo-license-policy-tests.log`, `tool-versions.txt` et `source-sha256.txt` gardent les commandes, sorties et empreintes. Le manifeste local et les index workspace/projet sont append-only; `append_manifests.py` capture leur préfixe antérieur et vérifie les hashes.
