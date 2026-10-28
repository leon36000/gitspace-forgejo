# CONTINUATION-172 — batch PR/commentaire, parent et reviewers partagés

Date : 26 octobre 2026. Dépôt de travail : `/home/pc6/Documents/Codex/2026-10-21-mission-construire-une-forge-con-ue/.runtime/webhook-005-workspace`, base HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. Le worktree était déjà fortement modifié; cette tranche a conservé les travaux existants. Seuls le test processus, son allowlist dans le runner, les documents de suivi et ce reçu changent pour CONT-172; aucune logique Rust de production ni dépendance Cargo n’a été modifiée.

## Scénario et observation

Le nouveau test `actual_reconcile_cli_process_keeps_reviewer_teams_on_pr_comment_batch_parent` (source ligne 7909) démarre `forgejo-webhook-local`, envoie deux enveloppes HMAC synthétiques à son socket : `pull_request` pour la PR 23, puis `issue_comment` avec `is_pull=true` pour le commentaire 915. Les deux livraisons sont acceptées en HTTP 202, en séquences 1 et 2; le receiver est arrêté avant le readback. Le journal contient deux entrées dont les octets sont ensuite comparés avant/après le CLI.

Le vrai processus `forgejo-reconcile-local` reçoit les deux IDs en une invocation. La fixture HTTP loopback sert deux réponses ordonnées : un GET de la PR parente, contenant un reviewer utilisateur et deux équipes avec des métadonnées privées superflues, puis un GET du commentaire. Assertions confirmées par le test :

- exactement deux GET, dans l’ordre parent puis commentaire, tous deux sans `Authorization`;
- schéma batch 8, verdict `current_state_read`, `upstream_read_attempts=2`, deux résultats dans l’ordre de livraison;
- les deux états canoniques conservent le même head, la même base et les mêmes reviewers projetés minimalement `{id,name}` et triés;
- le texte du commentaire est marqué `untrusted_user_content`; aucun e-mail, description d’équipe ou organisation privée ne fuit dans la sortie;
- le journal receiver demeure byte-for-byte identique.

Commande exacte :

```text
python3 scripts/run_webhook_reconcile_cli_e2e.py --scenario actual_reconcile_cli_process_keeps_reviewer_teams_on_pr_comment_batch_parent
```

Le test ciblé final passe `1 passed; 0 failed; 0 ignored; 128 filtered out`. Sortie complète : [`process-test-final.log`](process-test-final.log). Le runner l’exécute dans Bubblewrap 0.9.0 avec réseau et namespace PID non partagés, environnement vidé, source staged en lecture seule et `/tmp` isolé. Le test est `#[ignore]` dans la suite ordinaire, car il réserve le port fixe allowlisté `127.0.0.1:3130`.

## Contrôles et identité des sources

`cargo fmt --all -- --check`, `git diff --check`, `python3 -m py_compile scripts/run_webhook_reconcile_cli_e2e.py`, `python3 scripts/verify_cargo_license_state.py` et `cargo metadata --offline --locked --no-deps --format-version 1` réussissent. Les sorties vérifiées sont jointes; une première sortie rustfmt, limitée au wrapping de trois assertions du nouveau test, est gardée sous `rustfmt-initial-diff.log`, puis corrigée avant le run final. Aucun `cargo test --all-targets` n’a été répété. Les empreintes SHA-256 des sources, du manifeste et du lockfile sont dans [`source-hashes.txt`](source-hashes.txt); versions et isolation sont dans [`environment.txt`](environment.txt).

Readback Cargo : `agent-forge-core 0.1.0`, `license=null`, `license_file=null`, `publish=[]`, `decision=no_license_selected`, `redistribution_rights=not_assessed`, et métadonnée personnalisée `spdx_package_license_declared=NOASSERTION`. La garde ne choisit pas de licence et ne démontre aucun droit de redistribution.

## Limites

Le receiver et le CLI Rust sont les vrais binaires du projet, mais leur serveur API est un fixture loopback déterministe; aucun serveur Forgejo n’a été démarré. Ce scénario n’exerce pas le endpoint live de reviewers équipes, ne fournit pas de transaction/snapshot atomique face à une mutation concurrente après le readback et n’établit pas l’isolation d’un processus agent. Aucun modèle, AgentOS, Forge Code, credential de merge, PR distante, coût, service externe, production ou code upstream Forgejo/GitSpace n’a été utilisé. C’est un test de composition du batch, pas un parcours produit complet ni une mesure comparative. Les critères restants sont suivis dans [`docs/checkpoint.md`](../../docs/checkpoint.md) et la [matrice des refus](../../docs/refusal-evidence-matrix.md).
