# FORGEJO-STDIO-MCP-SAME-CANDIDATE-WEBHOOK-001 — run local

Date locale : 2026-10-26. Résultat : **réussi sur l’instance Forgejo de test**.

## Périmètre et provenance

Ce run reprend le runner local `scripts/run_forgejo_live_pr_merge_e2e.py` et l’option `--observe-pr-webhook`. Il utilise une nouvelle instance locale jetable dans `/tmp/forgejo-mcp-webhook-007`, le processus MCP encore vivant, l’autorité de fusion Rust, le receiver webhook local et le CLI de réconciliation. Le modèle est `local-test-adapter`, explicitement déterministe; aucun LLM ni AgentOS n’est configuré ou appelé.

Forgejo annonce `17.0.0+gitea-1.22.0`; l’empreinte du binaire est dans `result-summary.json`. La source officielle auditée précédemment est Forgejo v17.0.0 au commit [`0d791421f85ca13a63b665053d81ea765fbbc571`](https://github.com/forgejo/forgejo/blob/0d791421f85ca13a63b665053d81ea765fbbc571/modules/structs/hook.go), blob `9c7f1deed1c38eeb435ab976bb8429c5458c7f2b`. `PullRequestPayload.Review` est un pointeur sans option `omitempty`. Aucun code Forgejo n’est copié.

Hôte : Linux aarch64, Bubblewrap 0.9.0, Rust/Cargo 1.95.0, Python 3.12.3; Forgejo embarque Go 1.26.9. Les hashes des sources locales touchées et de Cargo.lock sont dans `source-hashes.json`.

## Commandes

```sh
python3 scripts/run_forgejo_live_pr_merge_e2e.py \
  --forgejo-binary /tmp/forgejo-17.0.0 \
  --state-dir /tmp/forgejo-mcp-webhook-007 \
  --non-admin-merger \
  --stdio-mcp-egress-during-merge \
  --observe-pr-webhook

cargo test --offline --locked --all-targets -- --test-threads=1 --quiet
/home/pc6/.venvs/qwen38-repair/bin/pytest -q \
  tests/test_forgejo_live_pr_merge_e2e.py \
  tests/test_forgejo_emitted_webhook_e2e.py
cargo fmt --all -- --check
git diff --check
```

## Résultat composé

Le même processus `--stdio-mcp` a échoué à l’acceptation initiale (5 tests rouges, notamment les bornes et `i32::MIN/MAX`), consommé le contexte et corrigé le fichier. L’acceptation protégée a alors passé 9/9. L’écriture du processus MCP, le candidat de PR, le vérificateur, le commit fusionné, le readback de `main` et la réconciliation webhook ont tous le SHA-256 `78cc4a0a2cafa1d15b62adc8782af05445e1708268d618bf970de3b2340d407a`. La PR #2 a été fusionnée en HTTP 200 sur le commit `2ac421ea44696b63494c6649399371464787de2f`; l’issue est fermée.

Forgejo a livré son événement `pull_request` / `closed` à la séquence 4. Le statut de la tâche Forgejo est réussi et le receiver répond HTTP 202. Le reconciler relit la PR courante, retourne `current_state_read` et lie `merge_commit_sha` au même `2ac421…`. La configuration du hook est ensuite désactivée et relue. `webhook-delivery-metadata.json` conserve uniquement les types d’événement/action, le type JSON du pointeur `review` et les états de livraison, jamais le payload brut.

Le live a d’abord reproduit le défaut : l’événement Forgejo avec `review: null` recevait un refus. Les deux tests rouges ciblés ont confirmé le 400 au receiver et `UnexpectedEventPayloadField("review")` au réconciler. La correction accepte `review` absent ou nul uniquement pour l’événement `pull_request`; toute valeur non nulle reste refusée avant append/GET. Pour `issues`, la présence de ce champ reste refusée. Les tests de contrat vérifient les deux côtés. La source wire et les réponses d’instance ont ainsi été testées ensemble.

Le faux statut de succès de l’exécuteur reste refusé; le candidat périmé est refusé; les merges sans approbation, sans whitelist ou par compte non-whitelisté sont refusés. Le processus enfant reste vivant à la barrière pendant la lecture de confiance. Les sondes isolées `api`, `web` et `git_receive_pack` restent bloquées; aucun credential de fusion n’est visible, hérité ou écrit dans un fichier accessible au processus. La lecture du gate et son merge autorisé réussissent. Après fusion, les 9 tests d’acceptation relancés sur les octets relus passent 9/9.

## Vérifications

`cargo-all-targets.log` : `cargo test --offline --locked --all-targets -- --test-threads=1 --quiet`, 579 réussis, 0 échec, 85 ignorés, en 34 exécutables de test. Les cas ignorés restent non couverts par cette commande. `python-targeted.log` : 91/91 pour les tests du runner de fusion et des webhooks Forgejo émis. Rustfmt et `git diff --check` passent.

## Limites

Un seul parcours, sur l’adaptateur déterministe; pas de modèle, AgentOS, RAG ou mémoire réels, pas de benchmark apparié, donc aucun gain général établi. Bubblewrap sur cet hôte et les trois routes sondées ne prouvent pas la sécurité contre une évasion noyau ou des writers Forgejo externes. Le run ne fournit pas de CAS serveur. Le canari de conflit sémantique intégré, la licence GitSpace et le choix/droits de redistribution du produit restent ouverts. Projet incomplet.
