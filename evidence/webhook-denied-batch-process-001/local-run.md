# WEBHOOK-DENIED-BATCH-PROCESS-001 — parcours receiver vers CLI sans lecture du credential

## Parcours testé

Le test ignoré par défaut `actual_reconcile_cli_process_denied_batch_does_not_read_host_credential_fd` traverse le vrai récepteur Rust et le vrai processus `forgejo-reconcile-local`. Deux événements synthétiques signés HMAC sont postés au receiver et acceptés en HTTP 202 : un payload sans référence de dépôt et une livraison visant `other-owner/other-repository`. Après arrêt propre du receiver, le processus CLI reçoit les deux IDs et un FD pipe contenant un token synthétique.

Sous Bubblewrap `--unshare-net`, le CLI rend un JSON batch `inconclusive` avec `invalid_repository` puis `upstream_forbidden`, `upstream_read_attempts=0` et un code de sortie d’échec normal. Après sa terminaison, le test lit le pipe côté parent et retrouve le token entier : le processus n’a pas consommé le credential. Le journal d’exécution montre build offline dans une cible temporaire puis exécution isolée de ce seul test : 1 réussi, 0 échec.

Le runner `scripts/run_webhook_reconcile_cli_e2e.py` accepte maintenant `--scenario` pour sélectionner un cas précis; son mode par défaut conserve toute la matrice. La commande de reproduction est :

```text
python3 scripts/run_webhook_reconcile_cli_e2e.py --scenario actual_reconcile_cli_process_denied_batch_does_not_read_host_credential_fd
```

## Contrôles connexes

`cargo fmt --all -- --check`, `python3 -m py_compile scripts/run_webhook_reconcile_cli_e2e.py` et `git diff --check` passent. Le garde Cargo confirme `license=null`, `license_file=null`, `publish=[]`, `license_decision=unselected`, `redistribution_rights=not_assessed`; `Cargo.toml` conserve `publish=false`. Aucune licence, clé Cargo, dépendance ou lockfile n’a été ajoutée ou modifiée.

La recherche des interfaces agent/modèle précédente reste dans [WEBHOOK-MIXED-CREDENTIAL-BATCH-001](../webhook-mixed-credential-batch-001/local-run.md); aucun provider n’a été appelé ou intégré ici.

## Limites

Receiver local et réconciliateur réel, mais toutes les données sont synthétiques. Le namespace réseau est isolé; aucun GET Forgejo/API n’est tenté, aucune instance Forgejo, PR, fusion, agent LLM ou service gateway n’est impliqué. Le test est `#[ignore]` dans Cargo par défaut et dépend du runner Bubblewrap. Il prouve le chemin de refus batch à la frontière du processus et la conservation du pipe; il ne prouve pas encore le batch mixte autorisé/hors périmètre depuis un processus CLI. Les résultats Cargo `--all-targets` antérieurs ne sont pas repris comme un run actuel.

Les sorties complètes, versions d’outils et hashes de sources sont conservés dans ce dossier et couverts par son `SHA256SUMS`.
