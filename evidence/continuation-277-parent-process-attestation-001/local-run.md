# CONTINUATION-277 — observation hôte de l’arbre agent au gate

Date : 27 octobre 2026  
Worktree : `.runtime/webhook-005-workspace`  
HEAD de base : `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0` (worktree fortement modifié; aucun commit créé).

## Changement

Quand les chemins privés de barrière sont fournis, le wrapper MCP attend maintenant le reçu prêt du superviseur de confiance. Il valide la forme, le propriétaire, les permissions, le PID du superviseur et le nonce du reçu. Tant que l’adaptateur reste suspendu sur l’arrêt MCP, le parent lit son arbre de processus depuis `/proc`; aucun champ d’espace de noms envoyé par l’adaptateur n’est utilisé comme seule preuve.

Le parent exige que le lanceur Bwrap reste dans ses namespaces hôte; ses descendants partagent des namespaces PID, réseau, montage et utilisateur uniques, distincts de l’hôte. Ils restent dans le cgroup de la racine, conservent le même UID effectif mappé, et un seul interpréteur Python local a les descripteurs `[0,1,2]`. Deux snapshots vérifient la stabilité. Le résultat du wrapper publie un reçu versionné; le runner mission le refuse si le reçu manque ou contredit les namespaces/fds du probe. Le résumé composé conserve le reçu.

## Vérifications

- TDD rouge avant implémentation : le test processus échouait car le reçu parent était absent; le test du validateur échouait car le validateur n’existait pas.
- `python3 -m unittest -v tests.test_mission_012_sandboxed_test_adapter tests.test_mission_012_live_mcp_agent_e2e` : **29 réussis, 0 échec** (`targeted-tests.log`).
- `python3 -m unittest -v tests.test_mission_012_live_composed_e2e` : **18 réussis, 0 échec** (`composition-tests.log`).
- `python3 evidence/continuation-277-parent-process-attestation-001/run_parent_procfs_probe.py` : held probe, observation parent avant/après reprobe identique, libération propre, code 0. Reçu : `process-observation.json`.
- `py_compile` des trois runners Python, deux modules de test et du runner de preuve; `git diff --check` : réussis. Un contrôle distinct des espaces finaux vérifie aussi les fichiers source non suivis : `whitespace-check.json`.
- `python3 scripts/verify_cargo_license_state.py` : réussi; métadonnées Cargo encore `license=null`, `license_file=null`, `publish=[]`, choix `no_license_selected`, droits `not_assessed`, SPDX `NOASSERTION` (`cargo-license-state.json`).
- Les commandes exactes et le résultat de la relecture de manifestes historiques sont dans `commands.txt`.

## Portée et réserves

Cette preuve couvre le `local-test-adapter` déterministe, tenu à la barrière dans un processus local Bubblewrap, et l’observation hôte de son arbre à cet instant. Le probe egress existant refuse les trois routes ciblées via un réseau non partagé; les tests de montage existants refusent l’accès aux fichiers privés choisis et l’écriture dans l’inbox canari. Aucun Forgejo live, modèle, AgentOS, secret réel, nouvelle PR, fusion ou benchmark n’a été lancé dans ce lot.

L’égalité de cgroup est une observation d’appartenance, pas la preuve indépendante que systemd applique toute sa politique. Le snapshot n’établit pas l’absence de processus créés après libération, l’exclusion de tous les writers, l’inaccessibilité de toute voie de récupération d’un credential, l’exhaustivité des montages accessibles, ni la résistance au noyau/root. Le même UID effectif est partagé. L’essai de lecture du FD parent n’a pas distingué la PID namespace d’un blocage ptrace local et n’est pas compté. Le test n’établit ni un parcours LLM ni un gain général.

Le re-vérificateur CONT-276 se réfère à l’ancienne empreinte source du wrapper et échoue donc après cette modification; il n’a pas été réécrit. Le vérificateur de ce lot compare les hashes actuels et confirme que les préfixes historiques worktree/projet sont inchangés.
