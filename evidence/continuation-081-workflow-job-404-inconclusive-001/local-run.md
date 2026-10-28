# CONTINUATION-081 — 404 du readback workflow_job

Date : 26 octobre 2026. Produit : worktree `/home/pc6/Documents/Codex/2026-10-21-mission-construire-une-forge-con-ue/.runtime/webhook-005-workspace`, branche `local/WEBHOOK-005-canonical-links`, base `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. Le checkout était fortement modifié. Ce lot ajoute un test de refus processus et la sélection de ce test dans le driver isolé; il ne change pas le comportement du binaire.

## Scénario et résultat

`actual_reconcile_cli_process_keeps_workflow_job_404_inconclusive_without_parent_read` soumet au receiver Rust réel un événement synthétique signé `workflow_job_success` pour le job `9001` du run `7001`. Le receiver accepte et journalise la livraison avec HTTP 202. Une fixture HTTP loopback répond ensuite 404 à l’unique GET du job dans le vrai processus CLI; la réponse inclut un marqueur privé qui ne doit pas être exposé.

Résultat observé : le CLI termine avec exit 1 et produit un résultat structuré `inconclusive`, code `resource_not_found`, `retryable=false`, un essai amont, et aucun `canonical_state`. L’URL demandée est exactement `/api/v1/repos/forge-admin/agent-forge-core/actions/jobs/9001`; aucun GET du run parent n’est émis. La requête ne comporte pas d’en-tête Authorization. Le texte du 404 n’apparaît pas dans stdout/stderr et le journal HMAC reste inchangé. Le conseil d’action précise que l’ID doit être vérifié et qu’un 404 ne distingue pas ressource absente et accès masqué.

Le parcours est lancé avec :

```sh
PYTHONDONTWRITEBYTECODE=1 python3 scripts/run_webhook_reconcile_cli_e2e.py \
  --scenario actual_reconcile_cli_process_keeps_workflow_job_404_inconclusive_without_parent_read
```

Le driver compile offline un arbre source en lecture seule, puis exécute le test sous Bubblewrap avec `--unshare-net`, `--unshare-pid`, environnement effacé et limites de processus. `process-e2e.log` indique 1 réussite et 96 tests filtrés.

## Vérifications et état licence

- `cargo test --offline --locked --all-targets -- --test-threads=1 --quiet` : 596 réussis, 0 échec, 89 ignorés, sur 34 exécutables de tests (`all-targets.log`, somme indépendante dans `test-summary.txt`).
- `cargo fmt --all -- --check`, `git diff --check` et `py_compile` du driver passent.
- `python3 scripts/verify_cargo_license_state.py` et `python3 -m unittest -v scripts.test_cargo_license_state` passent; la suite comporte 9 tests. Cargo reste `license=null`, `license_file=null`, `publish=[]`, `no_license_selected`, droits `not_assessed`, marqueur SPDX `NOASSERTION`.
- Les manifestes locaux et append-only produit/parent sont vérifiés après mise à jour; les préfixes antérieurs restent intacts. Les outils et leur version sont dans `tool-versions.txt`.

## Limites

`n=1`; seule l’API est simulée, dans le namespace réseau privé du test. Aucune instance Forgejo, aucun runner Actions, LLM, AgentOS, RAG, mémoire, donnée privée externe, coût fournisseur, publication ou production n’a été utilisé. Le résultat établit le refus d’inférer un succès d’un webhook quand le readback du job est inconclusif; il ne prouve pas qu’un 404 signifie que le job a été supprimé. Le commit Forgejo source et l’inventaire pertinent sont consignés dans [CONTINUATION-080](../continuation-080-workflow-job-readback-001/local-run.md); aucun code Forgejo n’est copié. Mission incomplète.
