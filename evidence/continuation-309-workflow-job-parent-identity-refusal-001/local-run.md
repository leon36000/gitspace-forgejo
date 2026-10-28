# CONTINUATION-309 — refus d’identité incohérente du workflow run parent

Date : 27 octobre 2026. Worktree : `.runtime/webhook-005-workspace`; HEAD de base `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. Le dépôt contient d’autres changements locaux; aucune remise à zéro ni modification de production n’a eu lieu.

## Cas exercé

Le webhook signé et le GET courant du job lient le job `9001` au run `7001`. Le CLI fait ensuite le GET exact `/api/v1/repos/forge-admin/agent-forge-core/actions/runs/7001`; la fixture répond HTTP 200 avec `id=7002`, alors que l’index/URL correspondent au run `7001`. Le code de réconciliation valide la réponse et refuse cette identité discordante. Le test appelle `run_workflow_job_parent_readback_refusal` du test processus existant, donc il traverse le vrai receiver, le journal durable, le vrai binaire reconciler et son adaptateur HTTP, sans inventer une fixture séparée.

Résultat observé : verdict `inconclusive`, erreur `resource_mismatch`, `retryable=false`, absence de `canonical_state`, deux GET dans l’ordre job puis run, aucun `/logs`, payload marqué privé non renvoyé et octets du journal identiques à ceux d’après l’acceptation. Le test est `actual_reconcile_cli_process_refuses_parent_run_identity_mismatch_after_valid_job_readback`.

## Exécution isolée

Commande :

```text
python3 scripts/run_forgejo_mission_delivery_process_e2e.py --only actual_reconcile_cli_process_refuses_parent_run_identity_mismatch_after_valid_job_readback
```

Le runner copie `Cargo.toml`, `Cargo.lock`, `src/`, `tests/` dans un répertoire temporaire borné, construit hors ligne avec le registre mis en scène, puis exécute uniquement ce test ignoré sous Bubblewrap `--unshare-net --unshare-pid`. Le test passe `1/1` (`167` filtrés). La commande, le source staged hash et les versions d’outils sont enregistrés dans `isolated-process.log`.

Environnement : `rustc 1.95.0 (59807616e 2026-04-14)`, `cargo 1.95.0 (f2d3ce0bd 2026-03-21)`, Bubblewrap `0.9.0`, Linux `7.0.0-1019-nvidia-aarch64-with-glibc2.39`; Cargo.lock SHA-256 `459fef3749a42c989521901dcde61cb26e12ed4206c5d722e149426c2d3a5439`. Le test a rapporté source staged SHA-256 `5565c9d3e091be1ab24ef78f2a407d6bd643bbae73b4b4ec2162565d10321ec3`, runner SHA-256 `ed36f0915b87e5903544c59cf377408b9764acd638c42472d4b7ce112069eb08` et registry SHA-256 `1f73c6a919948c0af44acac0604f8bf4f6ff4888379605e02d3976f1baea6451`.

## Portée

API HTTP synthétique locale, un cas (`n=1`), pas une instance Forgejo live ni Actions runner réel. Cela teste le refus d’une réponse parent liée à une autre identité; cela ne prouve pas une atomicité entre les deux GET, l’authenticité de Forgejo ou un verdict de CI. Le statut Actions est descriptif, jamais une preuve indépendante ou une autorisation de fusion. Aucun LLM/AgentOS, RAG ou mémoire n’est utilisé; aucun coût externe n’est engagé et aucun code Forgejo/GitSpace n’est copié.

## Références

- `tests/webhook_receiver_process_e2e.rs` — nouveau test ignoré, helper `run_workflow_job_parent_readback_refusal`.
- `scripts/run_forgejo_mission_delivery_process_e2e.py` — test listé pour exécution isolée.
- `docs/refusal-evidence-matrix.md`, `docs/source-inventory.md`, `docs/checkpoint.md` — couverture et état actualisés.
