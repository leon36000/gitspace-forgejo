# CONTINUATION-361 — refuser une édition d’issue pendant l’admission

Date : 28 octobre 2026. Worktree : `local/WEBHOOK-005-canonical-links`, base `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. Le dépôt est déjà très modifié; ce lot ne réinitialise, ne nettoie, ne commit, ne publie et ne pousse aucun fichier.

## Lacune et changement

`prepare_admission` lisait le snapshot courant d’une issue, puis récupérait le commit protégé et son inventaire d’exigences avant d’allouer une génération. Un changement du titre ou des labels pendant ces lectures pouvait être associé au snapshot précédent. Le test processus rouge `actual_mission_cli_refuses_issue_snapshot_change_during_admission_then_retries_current_state` a reproduit ce résultat : le CLI réclamait la génération 1 et produisait un handoff, alors que la fixture venait de modifier l’issue.

Le code relit maintenant `IssueSnapshotV1` après la validation des entrées protégées et avant `allocate_generation`. Une inégalité renvoie `mission_issue_changed_during_admission` avec une action de reprise; aucun événement de génération ni fichier de handoff n’est créé. Après stabilisation de la fixture, le retry du même delivery signé réclame la génération 1 et porte le titre et le label lus à l’API. Cette comparaison couvre l’intervalle effectivement testé; elle ne constitue pas une transaction avec Forgejo.

## Commandes et résultats

- Rouge (conservé dans `race-red.log`) : `python3 scripts/run_forgejo_mission_delivery_process_e2e.py --only actual_mission_cli_refuses_issue_snapshot_change_during_admission_then_retries_current_state` — échoue avant la correction car `claimed` est retourné au lieu d’un refus.
- Vert (conservé dans `race-green.log`) : même commande — 1/1; receiver/CLI et fixture tournent dans Bubblewrap avec `--unshare-net --unshare-pid`, les sources en lecture seule et la fixture loopback.
- Suite : `cargo test --offline --locked --all-targets -- --test-threads=1` — code 0, 38 groupes, 739 réussis, 0 échec, 167 ignorés (`all-targets.log`). Le run antérieur `all-targets-first-failure.log` a révélé un compteur d’issues de fixture devenu obsolète après les relectures; les attentes des chemins réussis ont été mises à jour avant le run final.
- `cargo fmt --all -- --check`, `git diff --check` et `python3 -m py_compile scripts/run_forgejo_mission_delivery_process_e2e.py` — code 0.
- `python3 scripts/test_cargo_license_state.py` — 12/12; `python3 scripts/verify_cargo_license_state.py` — `metadata_policy_verified`, `license=null`, `license_file=null`, `publish=[]`, `no_license_selected`, droits de redistribution `not_assessed`, export SPDX `NOASSERTION`.

Les versions d’outils, hashes des sources/logs et index append-only sont dans ce dossier. Aucun changement de dépendance ou choix de licence n’est introduit par ce lot.

## Portée et limites

Il s’agit d’un test processus local à fixture HTTP, `n=1`, avec clés synthétiques. Aucun Forgejo/GitLab live, email, LLM, AgentOS, PR, fusion ni comparaison de référence n’a été utilisé. Le full suite exclut 167 tests marqués ignorés; le cas nouvellement ajouté a été explicitement exécuté par son runner et n’est pas compté parmi les all-targets automatiques.

La deuxième lecture peut être immédiatement suivie d’une autre édition : sans ETag/CAS/transaction imposé par la source, l’allocation et le handoff ne sont pas atomiques avec l’état Forgejo. Un changement après ce readback reste hors garantie et ne peut pas être décrit comme protégé par un verrou distribué. Le refus ne valide ni les exigences ni la correction du code candidat.

## Prochaine action

Établir par interface/source vérifiable si un jeton de version ou une précondition conditionnelle de lecture existe pour l’issue et les exigences, puis décider si l’admission peut réellement être conditionnée à cette version. Si Forgejo ne fournit pas de primitive transactionnelle compatible, documenter cette limite et maintenir l’inconclusif comme choix prudent. La mission globale reste incomplète : parcours LLM réel, refus de sécurité restants et comparaison contrôlée n’ont pas été démontrés dans ce lot.
