# CONTINUATION-176 — refus hors scope conservé après 401 sur la PR configurée

Le runner n’émet pas d’horodatage d’exécution; l’environnement a été relevé le 2026-10-27 après le run. HEAD observé : `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`; worktree fortement modifié, non commité. Ce lot n’a modifié que le test processus, la liste des scénarios du runner et les documents de preuve; aucun code de production, manifeste Cargo ou lockfile n’a changé.

## Exécution

Commande exacte :

```text
python3 scripts/run_webhook_reconcile_cli_e2e.py --scenario actual_reconcile_cli_process_preserves_foreign_first_pr_refusal_after_configured_401
```

Le runner copie les sources sous `/tmp`, utilise le registre Cargo local en mode offline/locked, compile le test et les binaires requis, puis lance le scénario dans Bubblewrap 0.9.0 avec namespaces réseau et PID séparés. Environnement : Linux `aarch64-unknown-linux-gnu`, Cargo/rustc 1.95.0, Python 3.12.3. La sortie finale donne `1 passed; 0 failed; 0 ignored; 131 filtered out`, 0,18 s; le runner sort avec le code 0. Le journal brut est dans `process-test-final.log`.

## Observations vérifiées

Le vrai receiver HMAC accepte deux livraisons synthétiques HTTP 202 et le vrai CLI lit ensuite le journal. La première PR #23 référence `forge-admin/agent-forge-sibling`; la seconde référence `forge-admin/agent-forge-core`. Une fixture loopback répond HTTP 401 à l’unique GET configuré `/api/v1/repos/forge-admin/agent-forge-core/pulls/23`, avec un marqueur de corps synthétique. Le GET comporte le credential synthétique transmis par FD privé; aucune requête sibling ne part.

Le CLI sort en code 1 avec verdict batch `inconclusive`, `upstream_read_attempts=1`, sans état canonique. Le premier événement conserve `resource_out_of_scope`, `retryable=false`; le second est `upstream_unauthorized`, `retryable=false`. La fixture reçoit exactement un GET. Le token et le marqueur du corps 401 sont absents de stdout/stderr, le pipe est consommé et le journal demeure identique octet pour octet.

Ce scénario ajoute le cas d’échec d’authentification PR après le refus hors scope en tête. Les tests précédents couvraient le 401 sur une issue suivi d’une issue étrangère, puis le chemin PR avec credential valide; ils n’exerçaient pas la conservation des deux causes dans cette permutation PR.

## Vérifications et état licence

- `cargo fmt --all -- --check` passe.
- `python3 -m py_compile scripts/run_webhook_reconcile_cli_e2e.py` passe.
- `git diff --check` passe pour le diff suivi; le nouveau test Rust non suivi est vérifié par rustfmt et compilation/exécution du scénario.
- Le garde Cargo passe : `license=null`, `license_file=null`, `publish=[]`, `no_license_selected`, droits `not_assessed`, champ personnalisé `NOASSERTION`.
- `cargo metadata --offline --locked --no-deps --format-version 1` passe; le résumé package est joint.
- `source-hashes.txt` donne les SHA-256 des sources précises du test et du runner.

## Limites

La réponse 401 provient d’un serveur fixture loopback, pas d’une instance Forgejo. Le token et le corps d’erreur sont synthétiques; les namespaces ne prouvent pas l’isolement face à un processus extérieur disposant du même credential. Aucun appel LLM/AgentOS/Forge Code/RAG/mémoire, merge, CI complète, Forgejo upstream, service externe, coût modèle, production ou publication n’est impliqué. Aucun gain comparatif n’est démontré. La licence Cargo reste non choisie et les droits de redistribution ne sont pas évalués. Le projet demeure incomplet.
