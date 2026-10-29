# FORGEJO-ISSUE-REPOSITORY-META-PROCESS-001 — contrôle local

Date : 2026-10-28  
Worktree : `.runtime/webhook-005-workspace`  
HEAD de départ : `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`  
Plateforme : Linux aarch64 (`gx10-e526`, noyau `7.0.0-1019-nvidia`)  
Rust : `rustc 1.95.0 (59807616e 2026-04-14)`

## Question et source primaire

Forgejo officiel v17.0.0, commit `0d791421f85ca13a63b665053d81ea765fbbc571`, déclare dans `modules/structs/issue.go` :

- `RepositoryMeta.Owner string` avec `json:"owner"`;
- `Issue.Repo *RepositoryMeta` avec `json:"repository"`.

Le Git blob du fichier est `07ff3079de602403c23dfb2530ff5ee46fb0e747`; SHA-256 des octets : `e6cb7890f1d7045b2423465d1e452b6f91c4d71c746957aefc9bade286d10006`. `git diff --quiet <commit> -- modules/structs/issue.go` réussit dans le scratch `/tmp/forgejo-v17-base-cas-poc`, donc ce fichier précis n’est pas modifié dans l’arbre sale. La lecture est limitée à ce DTO et ne constitue pas un audit de tous les endpoints ni une observation du comportement d’une instance vivante.

## Parcours exécuté

Commande exacte :

```text
python3 scripts/run_webhook_reconcile_cli_e2e.py --scenario actual_reconcile_cli_process_reads_v17_issue_repository_metadata_owner_string
```

Le runner a compilé hors ligne sous `Bubblewrap` et lancé le test avec `--unshare-net --unshare-pid`, environnement effacé, sources en lecture seule, capacités de build bornées et loopback privé pour la fixture. Le scénario relie le vrai processus `forgejo-webhook-local`, son journal local et le vrai binaire `forgejo-reconcile-local`. Le corps webhook HMAC est synthétique; la réponse GET vient d’un serveur HTTP loopback de fixture, avec `repository.owner` sous forme chaîne.

Résultat observé : **1 réussi, 0 échec, 204 filtrés**, code de sortie 0. La sortie ciblée est dans [run-result.txt](run-result.txt).

Assertions du test :

- l’ingress accepte le webhook signé avec HTTP 202;
- la fixture répond à exactement un `GET /api/v1/repos/forge-admin/agent-forge-core/issues/41`;
- la forme `repository.owner: "forge-admin"` est acceptée et projetée avec le dépôt et le numéro attendus;
- le verdict est `current_state_read`, et l’état courant `open` remplace l’état périmé `closed` du webhook;
- aucun en-tête `Authorization` n’est envoyé;
- la commande de lecture ne modifie pas les octets du journal.

Le test refusant le dépôt étranger existe séparément; ce lot ne le relance pas. Le scénario nouveau est un complément de compatibilité positive, pas une correction de code runtime.

## Vérifications et empreintes locales

- `cargo fmt --all` : terminé avec code 0.
- `cargo fmt --all -- --check` : réussi avant l’exécution du scénario.
- `python3 -m py_compile scripts/run_webhook_reconcile_cli_e2e.py` : réussi.
- `git diff --check` : réussi avant la mise à jour de ce rapport; il est relancé après l’édition documentaire.
- `tests/webhook_receiver_process_e2e.rs` SHA-256 : `b65f7c2de55dc55a7275d75d435b5a87d64cefb402ee149a38676f9f3aeed703`.
- `scripts/run_webhook_reconcile_cli_e2e.py` SHA-256 : `d8be95bb8c01cbc1269fbf5689c26e315a8cc67dc94943ea55077a1d882204d7`.
- `Cargo.lock` SHA-256 : `459fef3749a42c989521901dcde61cb26e12ed4206c5d722e149426c2d3a5439`.

## Limites

Cette preuve établit la compatibilité de cette forme JSON à travers les processus Rust sous test. Elle ne prouve pas qu’un serveur Forgejo a émis la réponse, ne valide qu’un DTO et un chemin issue, et ne démontre ni l’authenticité d’une instance, ni la sécurité générale du sandbox, ni AgentOS/LLM, ni fusion, ni valeur comparative. Aucun code Forgejo/GitSpace n’a été copié; aucun service distant, secret, coût de modèle ou accès élargi n’a été utilisé.
