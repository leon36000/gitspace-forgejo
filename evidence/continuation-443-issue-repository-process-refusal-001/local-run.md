# CONTINUATION-443 — refus processus d’une identité de dépôt contradictoire

Date : 28 octobre 2026. Worktree : `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0` (checkout fortement modifié; aucun commit créé).

## Résultat

Ajout d’un scénario processus isolé, sans changement du code de production. La fixture API garde `repository.owner`, `repository.name`, `html_url` et la route cohérents avec le dépôt configuré, mais donne un `repository.full_name` divergent. Le receiver réel accepte la delivery HMAC synthétique (HTTP 202). Le vrai `forgejo-mission-local` répond `inconclusive / mission_source_invalid_response`, sans allouer de génération ni écrire de handoff; la réponse ne révèle pas la valeur étrangère. Après correction de l’état de la fixture, le retry de la même delivery réclame la génération 1, écrit les deux événements attendus et produit un handoff validable. L’inbox conserve une seule delivery.

## Vérifications

- `python3 scripts/run_forgejo_mission_delivery_process_e2e.py --only actual_mission_cli_refuses_mismatched_issue_repository_before_allocation_then_retries_corrected` : **1 réussi**, sous Bubblewrap avec namespaces réseau et PID isolés; le journal complet est `process-e2e.log`.
- `cargo test --locked --offline --bin forgejo-mission-local -- --test-threads=1` : **49 réussis**.
- `cargo fmt --all -- --check`, `git diff --check`, analyse AST du runner Python et `python3 scripts/verify_cargo_license_state.py` : passent.

Les empreintes du worktree source, de `Cargo.lock` et du runner, ainsi que versions d’outils et plateforme, sont dans `environment.txt`; commandes exactes dans `commands.txt`.

## Portée et limites

Le test traverse les processus réels receiver/CLI mais sert une API HTTP loopback synthétique dans une sandbox locale. `n=1`; il établit le refus du champ contradictoire et le retry dans ce scénario, pas l’authenticité d’un serveur Forgejo live, la cohérence atomique de lectures multiples, un parcours LLM/AgentOS, ni un gain comparatif. Il ne change pas le comportement runtime et ne revalide pas tout `--all-targets`.

## GitLab PC4

Le connecteur PC4 a confirmé en lecture seule le projet privé `root/gitspace-forgejo` (ID 4), la branche de travail et le commit `9ac877767e9b458a9cd911e67a10f87aeeaefbdb`; le pipeline 328 de ce commit reste en échec. Aucun artefact ni journal brut n’a été relu dans ce lot, aucune écriture distante n’a eu lieu. Le courriel n’a pas été ouvert : le connecteur existant suffisait, et le checkpoint signale qu’un ancien PAT ne doit pas être réutilisé. Le push reste suspendu jusqu’au traitement de l’alerte Gitleaks et du jeton d’upload CI potentiellement exposé, documenté dans `docs/checkpoint.md`.
