# CONTINUATION-292 — admission concurrente d’une même mission

Date d’exécution : 27 octobre 2026. Worktree : `/home/pc6/Documents/Codex/2026-10-21-mission-construire-une-forge-con-ue/.runtime/webhook-005-workspace`. HEAD observé : `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`; arbre de travail fortement modifié, aucun commit créé.

## Question vérifiée

Deux livraisons signées distinctes de la même issue peuvent-elles démarrer deux vrais processus d’admission concurrents sans créer plusieurs générations, claims ou handoffs? Si le verrou partagé refuse une tentative, le même ID de livraison peut-il reprendre après libération et converger sur l’état durable déjà revendiqué?

## Parcours

Le test `actual_mission_cli_concurrent_duplicate_deliveries_claim_one_generation_once` accepte deux corps HMAC identiques `issues/opened`, IDs `mission-concurrent-41-a` et `mission-concurrent-41-b`, pour `forge-admin/agent-forge-core#41`. Le receiver Rust réel ajoute deux entrées à l’inbox synthétique. Deux processus réels `forgejo-mission-local` sont démarrés par une barrière de threads. Le test tient le verrou du journal partagé jusqu’à ce que les deux aient lu la source de contexte au commit `0123456789abcdef0123456789abcdef01234567`, puis le libère.

Le run final constate deux refus `execution_mission_busy`, chacun `retryable=true` avec l’action de retry attendue. Il rejoue chacun des mêmes IDs contre la fixture toujours vivante. Les deux résultats finaux sont exactement un `claimed` et un `already_claimed`; ils sont liés à la même empreinte d’exigences, allocation `admission-41`, génération 1 et au même `MissionHostAdmissionV1` valide. Le journal contient exactement `mission_generation_allocated_v1` puis `mission_execution_claim_v1`; les deux événements webhook restent dans l’inbox. Le runner rapporte **1 test réussi, 0 échec** en 0,35 s.

Le runner compile puis exécute ce test `#[ignore]` dans Bubblewrap 0.9.0 avec `--unshare-net --unshare-pid`, source montée en lecture seule, runtime neuf en tmpfs, et réseau externe indisponible. L’API HTTP utilisée par les deux CLIs est une fixture loopback dans ce namespace, non une instance Forgejo.

## Itérations et corrections

- Les essais intermédiaires ont montré que l’issue sûre peut être `claimed` + `already_claimed` sans réponse `busy`; le test ne requiert donc plus artificiellement un refus de contention, et ne rejoue que les livraisons effectivement refusées. Le premier essai (`process-first.log`) et le résumé de l’essai suivant sont consignés ici; leur conclusion est conservée dans le checkpoint.
- `process-continued.log` a échoué avec `mission_source_unavailable`: le harnais arrêtait la fixture HTTP avant le retry qu’il voulait tester. La fixture reste maintenant active jusqu’à la fin des reprises; ce refus était causé par le test, pas par le CLI.
- `process-retry-final.log`, `process-continued-002.log` et les journaux de formatage conservent des erreurs intermédiaires d’édition/compilation. Elles sont résolues dans la source finale et ne sont pas comptées comme résultats verts.
- `process-continued-003.log` a passé le test avant le formatage final; `process-final.log` est le reçu ciblé du source formaté final et observe deux refus retryables avant la convergence. `tee` a réutilisé ce nom qui appartenait aussi à un ancien run échoué; les octets de cet ancien journal ont été remplacés. Le diagnostic ancien est résumé dans le checkpoint et ce rapport, mais son stdout original ne fait plus partie des reçus disponibles.

## Vérifications finales

- `python3 scripts/run_forgejo_mission_delivery_process_e2e.py --only actual_mission_cli_concurrent_duplicate_deliveries_claim_one_generation_once` : 1/1 passe dans le runner isolé.
- `cargo test --locked --offline --all-targets` : 694 passés, 0 échec, 154 ignorés, 37 blocs. Les scénarios processus ignorés par Cargo ne sont pas exécutés par cette commande; le test ciblé ci-dessus a été lancé séparément.
- `cargo fmt --all -- --check`, `rustfmt --check --edition 2024 tests/forgejo_mission_delivery_process_e2e.rs`, parsing AST du runner Python, `python3 scripts/verify_cargo_license_state.py` et `git diff --check` : exit 0.
- Le contrôle des métadonnées conserve `license=null`, `license_file=null`, `publish=[]`, `no_license_selected`, `redistribution_rights=not_assessed` et `NOASSERTION`; cela ne constitue pas une évaluation juridique des droits.

Outils : Rust/Cargo 1.95.0, Python 3.12.3, Bubblewrap 0.9.0, Linux 7.0.0-1019-nvidia-aarch64 avec glibc 2.39. HEAD, empreintes, commandes et sorties complètes sont dans les reçus voisins.

## Portée et limites

Cette preuve relie receiver signé, inbox durable, admission CLI, verrou/journal, allocation, claim et handoff pour deux livraisons concurrentes dans un bac à sable local. Un seul run, une seule fixture et une même machine Linux/aarch64 sont testés. Elle ne prouve ni la concurrence entre plusieurs hôtes ou systèmes de fichiers distribués, ni la sémantique de writers et de branches d’un Forgejo live, ni un conflit de code ou une détection générale de conflits sémantiques. Aucun LLM/AgentOS, service externe, credential réel, déploiement, modification du runtime ou comparaison de productivité n’est impliqué.

Le lot améliore une preuve de reprise concurrente; il ne valide pas tous les critères cumulatifs et ne démontre aucun gain général. La mission reste incomplète.
