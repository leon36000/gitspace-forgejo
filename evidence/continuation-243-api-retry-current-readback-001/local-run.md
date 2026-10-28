# CONTINUATION-243 — retry transitoire puis readback courant

Date : 27 octobre 2026. Worktree local préexistant `.runtime/webhook-005-workspace`, HEAD de base `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. Aucun reset, commit, service de production, publication ou appel externe.

## Résultat observé

Le scénario `actual_reconcile_cli_process_recovers_from_in_budget_retry_after` suit le même processus de bout en bout : le receiver Rust réel accepte un webhook HMAC dont le snapshot indique l’issue `closed`; le CLI de réconciliation réel lit ensuite le journal durable et interroge l’API HTTP de fixture. Celle-ci répond `429` avec `Retry-After: 0`, puis `200` avec l’état canonique courant `open`. Le résultat du CLI est `current_state_read`, `upstream_read_attempts=2`, et l’état projeté est `open`, non l’état ancien du webhook.

Les assertions vérifient deux GET sur la route d’issue attendue, aucune ligne `Authorization`, l’absence du marqueur du corps de 429 dans la sortie, stderr vide, et une égalité octet pour octet du journal avant et après. Le scénario passe sur la source finale sous Bubblewrap avec `--unshare-net --unshare-pid`; le wrapper compile les cibles offline puis exécute un test ignoré explicitement dans cette sandbox. Résultat : 1 passé, 0 échec, 148 filtrés.

## Modifications de ce lot

Ajout du test de processus et de son nom à la liste fixe du runner E2E déjà présent dans le worktree. Le runtime de receiver et de réconciliation n’a pas été modifié dans ce lot. Le contrôle Rustfmt initial a trouvé une seule mise en forme de `assert_eq!`; elle a été corrigée, puis le contrôle final passe. Le journal initial et son code de sortie sont conservés, avec le journal du run ciblé final.

## Portée et limites

La fixture HTTP est synthétique et loopback; il ne s’agit pas d’une réponse d’une instance Forgejo live. `Retry-After: 0` ne démontre pas une attente positive ou le comportement sous charge. `n=1`, événement issue seulement. Aucun modèle, AgentOS, runner, parcours de PR/fusion ou comparaison avec CI conventionnelle n’est exercé. Cette preuve renforce un seul chemin adapter; elle ne prouve pas de gain de valeur. La mission reste incomplète.

Les fichiers `commands.txt`, `source-review.txt`, `tool-versions.txt`, `source-sha256.txt`, les sorties et le manifeste local rendent le run vérifiable. Le manifeste append-only valide la préservation des préfixes des index worktree et projet.
