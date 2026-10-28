# Run local 036 — reçus de réconciliation PR liés aux livraisons

Date : 24 octobre 2026. Worktree `HEAD` source : `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`, avec un grand nombre de modifications locales antérieures conservées. Les SHA-256 exacts des sources et binaires consommés sont dans `composed-run.json` et les reçus associés. Forgejo local `17.0.0+gitea-1.22.0`, binaire `/tmp/forgejo-17.0.0`, SHA-256 `e5ec437cd6357e8d1b4580cb47ab1daa58a32b33971a63db63446a535d33adb7`. Le clone officiel v17 examiné est au commit `0d791421f85ca13a63b665053d81ea765fbbc571`; la provenance de construction de ce binaire précis depuis ce clone n’est pas établie.

Commande reproductible :

```sh
python3 scripts/run_mission_012_live_composed_e2e.py \
  --forgejo-binary /tmp/forgejo-17.0.0 \
  --state-dir /tmp/mission012-composed-adapter-036 \
  --evidence-dir "$PWD/evidence/mission-012-live-agent-webhook-local-adapter-036" \
  --execution-id live-mcp-036 \
  --allocation-id mission012-local-adapter-036 \
  --budget-cap-usd 0.20 \
  --agent-adapter local-test-adapter
```

## Résultat observé

- Forgejo a livré un événement `issues/opened` local, lié à l’exigence versionnée SHA-256 `3a3c444075c93a3bcfb2bec17d1a2e74ec0051effc2994e6b02ac095eb0dd431`, à l’allocation `mission012-local-adapter-036` et à la base `184a73dfd54d73f4ef7e2874ff13eb73a03eb995`. Le replay d’admission est idempotent; un credential invalide est refusé avant création de l’admission.
- L’adaptateur déterministe consomme les outils `mission.context`, `workspace.read_text`, `workspace.write_text` et `cargo.test`. L’acceptance échoue d’abord sur la borne haute (`left: -2`, `right: 10`); le candidat ne modifie que `src/lib.rs` et passe les deux tests d’acceptance.
- La PR locale #2 est liée à la base et au candidat `c29b3ea38058e2bfbff606d49a09949b6c7b2666`. Le vérificateur indépendant passe. Forgejo refuse le merge avant revue et la tentative de fusion du reviewer de test `verifier-bot` en HTTP 405. La revue approuvée est relue séparément par l’API et liée au candidat; elle n’est pas déduite du webhook.
- L’autorité de fusion Rust fusionne puis relit le commit `c3a11bad9fd8e8602a5f4047dbc312f924600e61`. Les notifications `opened`, `pull_request_approved/reviewed` et `closed` ont chacune une tâche Forgejo, une livraison distincte et HTTP 202. Le reçu v4 conserve trois résultats de réconciliation, chacun lié à son UUID, au PR #2, au verdict `current_state_read`, à une lecture amont et à l’état courant fermé/fusionné avec le candidat et le commit de fusion exact. Les séquences observées sont 1, 2 et 3. Le hook est ensuite désactivé avec readback; le receiver s’arrête avec succès et aucun listener ne subsiste.
- Le hook n’utilise pas les notifications de revue comme preuve d’approbation. Le test valide aussi l’absence de reçu, les doublons ou mauvais UUID, une sortie en échec, une base périmée et un booléen JSON remplacé par l’entier `1`.

Les reçus détaillés sont `composed-run.json`, `composed-stage-summaries.json`, `pr-webhook-reconciliation.json`, `forgejo-result.json`, `independent-verifier.json`, `premerge-reverification.json` et `rust-merge-authority-result.json`.

## Vérification locale

- `cargo test --offline --locked` : 387 réussis, 0 échec, 14 ignorés, sur 33 groupes de tests.
- `python3 -m unittest tests.test_mission_012_live_composed_e2e tests.test_mission_012_live_agent_pr_e2e tests.test_forgejo_emitted_webhook_e2e` : 63 tests réussis.
- `python3 -m py_compile` sur les deux drivers et le test de composition, `cargo fmt --all -- --check`, Clippy `--all-targets` avec les trois allowances historiques (`too_many_arguments`, `bool_assert_comparison`, `manual_div_ceil`) et `git diff --check` : succès.
- `cargo metadata --offline --locked --format-version 1` confirme `license=null` pour `agent-forge-core`; la preuve est `cargo-license.json`.
- Les sorties de vérification sont conservées dans les fichiers `.log`; les empreintes sont dans les deux manifestes `SHA256SUMS` de ce run et de son admission.

## Portée et limites

Il s’agit d’un parcours `n=1` avec `local-test-adapter`; aucun LLM n’a été appelé, coût modèle observé `0 USD`. Ce résultat ne mesure pas un gain face à un agent et une CI conventionnels. AgentOS, Forge Code, RAG et mémoire ne sont pas intégrés par ce parcours. Bubblewrap isole le vérificateur, mais les services partagent l’UID; le verrou de fusion ne protège que les writers coopératifs et Forgejo ne fournit pas de CAS de base contre un writer externe. Le binaire Forgejo est identifié par hash, mais sa chaîne de construction depuis le clone officiel n’est pas attestée. Le compte owner et `verifier-bot` sont des identités locales de fixture. `license=null` reste inchangé; ce run n’accorde ni ne conclut aucun droit de réutilisation ou redistribution GitSpace.

Le contrôle Sonnet strictement passif trouve toujours le PID Claude CLI `1947069`, sans changement des mtimes suivis (`ralph-loop.local.md`, `local-run.md`, `SHA256SUMS`). Aucun argv, prompt, environnement, log, transcript ou terminal n’a été consulté; aucun signal ni redémarrage n’a été effectué.
