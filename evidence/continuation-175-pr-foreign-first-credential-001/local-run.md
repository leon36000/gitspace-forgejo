# CONTINUATION-175 — credential après refus d’une PR étrangère en tête de batch

Date du run : 2026-10-26. HEAD observé : `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`; worktree fortement modifié, non commité. Le test et son runner étaient des ajouts non suivis. Aucun code de production, manifeste Cargo ou lockfile n’a été modifié par cette tranche.

## Question vérifiée

Lorsqu’un batch du dépôt configuré contient d’abord une livraison signée pour une PR de même numéro dans un dépôt frère, est-ce que le CLI refuse cette ressource sans abandonner le batch, puis utilise le credential fourni uniquement pour lire la PR configurée? Les assertions incluent le chemin HTTP exact, le nombre de requêtes, l’authentification, l’absence du dépôt frère dans la requête, l’absence du token dans stdout/stderr et l’immutabilité du journal.

## Exécution et résultat

Commande :

```text
python3 scripts/run_webhook_reconcile_cli_e2e.py --scenario actual_reconcile_cli_process_uses_credential_only_after_refusing_foreign_first_pr
```

Le runner a copié le source dans un répertoire temporaire en lecture seule et compilé offline/locked les cibles requises. Le test s’est exécuté sous Bubblewrap 0.9.0 avec `--unshare-net --unshare-pid`, les threads Rust réglés à 1, et limites de processus/temps configurées par le runner. Rust/Cargo 1.95.0, cible `aarch64-unknown-linux-gnu`, Python 3.12.3, noyau Linux 7.0.0-1019-nvidia. Sortie finale : `1 passed; 0 failed; 0 ignored; 130 filtered out` en 0,09 s; code de sortie du runner : 0. Le reçu complet est `process-test-final.log`.

Le vrai receiver HMAC accepte deux événements synthétiques HTTP 202 puis s’arrête; le vrai `forgejo-reconcile-local` lit le journal. Le premier événement vise `forge-admin/agent-forge-sibling`, le second `forge-admin/agent-forge-core`, tous deux PR #23. Un token **synthétique** est transmis au CLI par pipe privé. La fixture HTTP authentifiée capture exactement un GET vers `/api/v1/repos/forge-admin/agent-forge-core/pulls/23`. Elle confirme le token sur cette route; aucune route sibling n’est appelée. Le premier résultat est `inconclusive/resource_out_of_scope` sans état canonique; le second est `current_state_read` avec head, base et reviewers attendus. Le batch reste globalement inconclusif à cause du refus hors scope, le token n’apparaît pas dans stdout/stderr, le pipe est consommé et l’inbox est byte-for-byte inchangée.

## Vérifications de cohérence

- `cargo fmt --all -- --check` passe après formatage du nouveau test.
- `python3 -m py_compile scripts/run_webhook_reconcile_cli_e2e.py` passe.
- `python3 scripts/verify_cargo_license_state.py` retourne `status=verified`, `license=null`, `license_file=null`, `publish=[]`, `no_license_selected`, `redistribution_rights=not_assessed`, marqueur personnalisé `NOASSERTION`.
- `cargo metadata --offline --locked --no-deps --format-version 1` passe; le résumé d’identité package est dans `cargo-metadata-summary.json`.
- Hashes SHA-256 des deux sources : `source-hashes.txt`.

Le premier contrôle rustfmt avant `cargo fmt` a signalé seulement les formes de mise en page du nouveau test; elles ont été corrigées, puis la vérification finale ci-dessus a réussi. Aucun défaut fonctionnel n’a été détecté.

## Portée et limites

La réponse API est un serveur fixture loopback, pas Forgejo live. Le credential est une chaîne synthétique, pas un secret utilisateur. Le résultat établit le routage du client et l’absence d’exposition du token dans ses sorties dans ce scénario; il ne prouve pas une ACL serveur, l’isolation réseau d’un autre processus ayant le même credential, une capacité multi-dépôts, une course concurrente ou une frontière contre root. CONT-174 couvre déjà l’ordre inverse sans credential; ce test ajoute uniquement le cas credential + ressource étrangère en premier.

Ni Forgejo/GitSpace upstream, LLM/AgentOS/Forge Code/RAG/mémoire, merge, CI complète, production, publication, dépense modèle ou service externe n’ont été utilisés. Aucun gain comparatif n’est mesuré. La licence produit n’est pas choisie et les droits de redistribution ne sont pas évalués; le projet reste incomplet.
