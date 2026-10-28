# CONTINUATION-286 — probe de la graine d’attestation depuis le processus agent

Date : 27 octobre 2026. Worktree local fortement modifié, HEAD de base `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. Ce lot n’a pas fait de commit, publication, appel de modèle, dépense ni modification de production.

## Lacune traitée

CONT-285 prouvait l’isolation réseau, les routes Forgejo ciblées et le credential de fusion, mais le rapport signalait qu’aucun appel `open` ne ciblait la graine Ed25519 réellement utilisée par le receiver. Le runner transmet maintenant au processus stdio-MCP sandboxé le chemin de cette même graine. Avant lancement, le wrapper vérifie qu’il s’agit d’un fichier régulier non symlink appartenant à l’UID courant, lié une seule fois, de mode `0600`, exactement 32 octets, sous `/tmp`, avec parent propriétaire de mode `0700`. Le probe, exécuté dans le namespace Bubblewrap qui masque `/tmp`, tente d’ouvrir le chemin en lecture et échoue fermé; le reçu n’expose que `webhook_attestation_seed`, jamais les octets ni leur empreinte.

Le test held-probe a d’abord échoué après ajout du nouveau cas parce que l’allowlist du sandbox le refusait (`ValueError: private runtime file probe must identify an absolute private file`). Après ajout du contrat de probe et du validateur de fichier, le processus sandboxé réussit et conserve la nouvelle étiquette dans `private_runtime_files_not_visible`. Le test valide aussi le refus du fichier en mode trop permissif et d’une taille différente de 32 octets.

## Parcours Forgejo local

Une instance locale temporaire a été créée dans `/tmp/forgejo-attestation-seed-probe-cont286-20261027-a`, à partir du binaire `/tmp/forgejo-17.0.0` (`17.0.0+gitea-1.22.0`, SHA-256 `e5ec437cd6357e8d1b4580cb47ab1daa58a32b33971a63db63446a535d33adb7`). Le commit de construction du binaire n’est pas établi. Run : `--non-admin-merger --stdio-mcp-egress-during-merge --observe-pr-webhook`.

Le run réussit la petite tâche fixture avec `local-test-adapter` déterministe : le premier test d’acceptation donne un échec actionnable (`exit 101`), puis la correction passe 9 tests réels avec `cargo test --offline --locked` (`exit 0`). Le processus agent est vivant à la barrière pendant l’appel du gate. Le reçu `stdio-mcp-egress-during-merge.json` établit :

- `private_runtime_files_not_visible=["webhook_attestation_seed"]` pour la vraie graine du receiver;
- aucun réseau connecté, routes API/Web/`git-receive-pack` bloquées, aucun credential visible, descripteurs `[0,1,2]` et zéro connexion au listener observée par le parent;
- POST du gate host HTTP 200 avec verdict `merged` pendant que l’agent est à la barrière;
- PR fusionnée par un acteur non administrateur; `main` relu égale le commit de fusion;
- lecture après fusion de 9 tests, tous réussis, sur la même source candidat `6937c269b5a838c3eb28343721f59774b3d9a15bac9da5016d788fa8e0005c04`;
- livraison native `pull_request/closed` réconciliée avec le commit de fusion (`current_state_read`); le reçu n’enregistre pas le secret du webhook.

Le log du run et un résumé filtré sont conservés ici. L’état Forgejo sous `/tmp` reste conservé et contient des credentials et la graine synthétiques; il n’a pas été copié dans le dossier d’évidence ni inspecté pour ses octets privés.

## Vérifications

- `python3 -m unittest discover -s tests -p 'test_mission_012_sandboxed_test_adapter.py'` : 15 tests réussis.
- `python3 -m unittest discover -s tests -p 'test_mission_012_test_agent_adapter.py'` : 9 tests réussis.
- `python3 -m unittest discover -s tests -p 'test_forgejo_live_pr_merge_e2e.py'` : 17 tests réussis.
- `python3 -m py_compile` sur les quatre runners/adaptateurs concernés et le test sandbox : code 0.
- `git diff --check` : code 0.
- Aucun fichier Rust n’a changé; `cargo test` Rust général n’a donc pas été relancé. Les 693 tests/151 ignorés du checkpoint 285 restent une preuve antérieure, pas un retest de ce lot.

Versions : Python 3.12.3; Bubblewrap 0.9.0; Linux aarch64, noyau `7.0.0-1019-nvidia`; Rust/Cargo 1.95.0 pour les tests Cargo de la fixture. `go` n’est pas disponible dans le shell hôte; le binaire Forgejo annonce `go1.26.9` dans ses métadonnées.

## Portée et limites

Le probe lie la preuve au fichier de graine concret et au processus d’agent réel de ce parcours, mais la preuve est limitée à un run (`n=1`), au `local-test-adapter`, à cette configuration Bubblewrap Linux/aarch64 et à un secret host sous `/tmp`. Elle n’établit pas l’isolation contre root, un hôte/noyau compromis, des montages différents ou d’autres OS. Le run conserve `external_writers_excluded=false`; Forgejo n’impose pas ici une précondition atomique universelle à tous les writers. Le parcours LLM/AgentOS, l’inventaire complet des licences et la comparaison appariée avec Forgejo + agent + CI conventionnels restent ouverts. Aucun gain général n’est déduit.

## Artefacts

`commands.txt`, `tool-versions.txt`, `forgejo-live-run.log`, `run-summary.json`, `sandbox-seed-probe-summary.json`, les journaux Python, le log `git diff --check` et `source-sha256.txt` conservent les commandes, résultats filtrés et empreintes de ce lot. `SHA256SUMS` couvre le dossier de preuve; `append_manifests.py` conserve le préfixe puis ajoute les hashes dans les index globaux append-only.
