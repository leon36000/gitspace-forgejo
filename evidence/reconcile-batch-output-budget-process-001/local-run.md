# RECONCILE-BATCH-OUTPUT-BUDGET-PROCESS-001 — refus confirmé par les processus receiver et CLI

Date : 27 octobre 2026. Worktree : `/home/pc6/Documents/Codex/2026-10-21-mission-construire-une-forge-con-ue/.runtime/webhook-005-workspace`. Base : `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`.

## Parcours exécuté

Le test isolé `actual_reconcile_cli_process_refuses_repeated_comment_batch_output_budget` démarre le vrai receiver Rust, envoie 100 requêtes webhook HMAC signées par un secret synthétique temporaire avec IDs distincts mais le même issue/commentaire, arrête le receiver, puis lance le vrai `forgejo-reconcile-local`. Le serveur API est une fixture privée loopback dans la même exécution Bubblewrap, qui utilise `--unshare-net` et `--unshare-pid`; aucun Forgejo externe ni service partagé n’est visé.

La fixture répond au GET de l’issue parente et au GET du commentaire avec un corps courant de 128 KiB. Le CLI sort en code 1 avec schéma batch 9, `verdict=inconclusive`, `batch_result_budget_exceeded`, `retryable=false`, 100 demandes et deux lectures amont. Il n’émet aucun tableau `results` ni état partiel; stdout reste inférieur à 16 KiB et ne contient pas une tranche du corps courant. La fixture ne reçoit que les deux GET canoniques (parent et commentaire), sans en-tête Authorization. Le journal signé est identique octet pour octet avant et après la tentative.

Commande reproductible :

```text
python3 scripts/run_webhook_reconcile_cli_e2e.py --scenario actual_reconcile_cli_process_refuses_repeated_comment_batch_output_budget
```

Le script compile hors réseau les cibles Rust depuis une copie bornée des sources et du cache Cargo, puis exécute exactement un test ignoré dans Bubblewrap. La sortie positive confirme 1 réussi, 0 échec; elle est dans `process-green.log`. Le premier essai `process-red.log` a bien exécuté le test, mais s’est arrêté au helper de test qui attendait schéma batch 8 alors que le binaire courant renvoie 9. La correction garde single=8 et batch=9; elle ne modifie pas le comportement CLI.

`cargo test --offline --locked --all-targets` après ces changements passe 658 tests, zéro échec, 122 ignorés sur 36 cibles. La nouvelle E2E est marquée ignorée par Cargo et exécutée séparément par son lanceur isolé; elle n’est pas comprise dans ces 658 réussites. Le pipeline fixture reste à 4/4. `cargo fmt --all -- --check`, `git diff --check` et l’analyse AST Python du lanceur passent.

## Portée et limites

Preuve de processus intégrée (receiver → journal → CLI → API synthétique) et de refus sans sortie partielle; pas d’instance Forgejo live, de charge soutenue, de test multi-hôte, de nouvel appel LLM, d’AgentOS/RAG/mémoire ou de benchmark de valeur. La fixture fixe le comportement et sa réponse de 128 KiB; `upstream_read_attempts` est corroboré par les deux requêtes observées. L’autorité de merge, les CAS Forgejo universels et l’intégration de production ne sont pas exercés. La réserve de 2 048 octets continue d’être un estimateur documenté de l’enveloppe CLI, pas un budget stdout mesuré à l’octet.

Cette tranche ne change aucun champ de licence. La vérification Cargo est jointe pour relire `license=null`, `license_file=null`, `publish=[]`, `no_license_selected`, droits `not_assessed`, SPDX `NOASSERTION`. Les hashes des sources, le toolchain, la sortie du garde et les préfixes des manifestes append-only sont dans ce dossier.
