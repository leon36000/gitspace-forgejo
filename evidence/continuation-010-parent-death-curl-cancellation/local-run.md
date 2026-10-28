# CONTINUATION-010 — annulation du curl enfant avec la mort du CLI parent

Date : 25 octobre 2026. HEAD de référence inchangé : voir head.txt. Le worktree est déjà fortement modifié et reste non commité; ce rapport attribue seulement les changements et essais de cette tranche.

## Lacune et preuve rouge

L’adaptateur attend une réponse HTTP dans un processus curl enfant. La suite testait l’expiration de son budget et une interruption où le test tuait explicitement tout le groupe de processus, mais pas le cas où un superviseur envoie SIGTERM au seul PID du CLI. Le nouveau test reconcile_cli_terminates_curl_child_when_parent_is_signalled observe le PID curl dans /proc, interrompt seulement le CLI et vérifie la vie du curl.

Avant la correction, le vrai test processus a échoué à l’assertion après 2,07 s : l’enfant curl était encore en cours, la réponse HTTP attendait toujours et la lecture persistante était donc toujours active. La sortie est dans red-run.log. Le harnais est sous Bubblewrap avec namespaces réseau et PID isolés; un garde de groupe dédié nettoie l’enfant également sur le chemin rouge.

## Correction

Sur Linux, run_command_bounded_with_input configure maintenant le curl avec PR_SET_PDEATHSIG(SIGKILL) avant exec. Le hook compare getppid au PID du CLI capturé avant fork; si le parent meurt avant que le signal puisse être installé, le démarrage de curl échoue au lieu de continuer sans la protection. SIGKILL empêche curl d’ignorer le signal et d’effectuer d’autres requêtes après la disparition du CLI. Les autres Unix gardent le comportement antérieur et annoncent explicitement not_guaranteed.

La découverte forgejo-reconcile-capabilities-v2 ajoute process_lifecycle.curl_child_parent_exit. Le readback du binaire réel est dans capabilities.json. Le test de processus vérifie que l’enfant n’est plus vivant ou zombie après le signal du seul parent et que le journal webhook reste inchangé.

## Commandes et résultats

- Rouge, avant correction : python3 scripts/run_webhook_reconcile_cli_e2e.py --scenario reconcile_cli_terminates_curl_child_when_parent_is_signalled — 0 réussi, 1 échoué à l’assertion de survie du curl; red-run.log.
- Vert, après correction : même commande — 1 réussi, 0 échoué sous Bubblewrap; green-run.log.
- cargo test --offline --locked --bin forgejo-reconcile-local capabilities_list_exact_merge_target_profiles -- --nocapture — 1 réussi; unit-capabilities.log.
- cargo test --offline --locked --test forgejo_reconcile_capabilities_e2e actual_cli_discloses_versioned_local_test_capabilities -- --nocapture — 1 réussi; process-capabilities.log.
- cargo build --offline --locked --bin forgejo-reconcile-local — réussi; build.log.
- cargo fmt --all -- --check, python3 scripts/test_webhook_reconcile_runner_sandbox.py, python3 -m py_compile scripts/run_webhook_reconcile_cli_e2e.py et git diff --check — réussis; journaux du même nom.
- python3 scripts/verify_cargo_license_state.py — vérifié, sans champ Cargo license/license_file, publish=[], no_license_selected, redistribution_rights=not_assessed et métadonnée SPDX NOASSERTION; license-state.log.

L’empreinte binaire, les sommes SHA-256 des sources, HEAD, versions et logs sont conservés ici. Environnement : voir tool-versions.txt; le binaire final est vérifié par binary-sha256.txt. Le manifeste de ce dossier couvre tous les autres fichiers et passe sha256sum -c SHA256SUMS.

## Limites

Le mécanisme est Linux-only et protège le processus curl direct; il n’arrête pas tous les descendants d’un superviseur et ne remplace pas l’annulation du groupe par le parent d’orchestration. Le CLI tué ne peut pas renvoyer un résultat structuré d’annulation; les événements durables restent inchangés et peuvent être relus par un nouveau processus. Il ne crée aucune séparation de privilèges entre un agent et le CLI porteur de credentials.

Le test emploie une issue et une signature HMAC synthétiques, un journal temporaire et une fixture HTTP loopback; le namespace réseau est isolé. Aucun Forgejo live, credential réel, LLM, AgentOS, service externe, coût, dépôt produit supplémentaire ou licence n’a été utilisé. Le parcours modèle et la comparaison conventionnelle appariée restent non démontrés.

La primitive et ses limites suivent Linux man-pages 6.19, PR_SET_PDEATHSIG, lues le 25 octobre 2026 : https://man7.org/linux/man-pages/man2/pr_set_pdeathsig.2const.html. La page précise que le signal de parent death est Linux-only, concerne la terminaison du thread parent créateur et n’est pas envoyé si celui-ci a déjà terminé avant l’installation; le contrôle getppid couvre cette dernière course dans le chemin testé.
