# CONTINUATION-311 — phase de refus des sondes directes vers Forgejo

Date : 27 octobre 2026. Worktree produit fortement modifié, HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. Le changement est limité au harnais Python de sandbox et à ses tests; aucun code Forgejo, noyau Rust produit, `Cargo.toml` ou `Cargo.lock` n’a changé.

## Lacune et correction

CONTINUATION-307 classait comme « bloquée » une route dès que `socket.create_connection()` **ou** `sendall()` levait `OSError`. Un échec d’envoi prouve qu’une connexion a été obtenue et ne suffit pas à prouver que le processus était isolé au niveau réseau. Le reçu 307 signalait explicitement qu’il n’enregistrait pas la phase ni l’errno.

`probe_sandbox_capabilities` rapporte maintenant, pour chaque sonde, seulement `{name, phase, error_category, errno}`; aucune chaîne d’exception n’est retournée. Le wrapper de confiance et le runner composé exigent l’ensemble ordonné API/Web/receive-pack, une erreur OS numérique positive, et `phase=connect`. Un échec à `send` reste distinct et fait refuser le reçu de sandbox.

La séquence TDD observée : test rouge car le champ diagnostique n’existait pas; test rouge car les validateurs wrapper/gate n’existaient pas; premier test processus d’intégration rouge parce que le wrapper rejetait le nouveau champ avant de le vérifier. Après les changements minimes du harnais, les deux suites ciblées sont vertes. Aucun défaut du code Rust ou Forgejo n’a été observé dans ces échecs de harnais.

## Vérification locale

- `python3 tests/test_mission_012_sandboxed_test_adapter.py` : **17/17**.
- `python3 tests/test_forgejo_live_pr_merge_e2e.py ComposedStdioFixtureConsistencyTests` : **3/3** après ajout du test qui vérifie le rejet d’un échec réseau après connexion.
- `python3 tests/test_forgejo_live_pr_merge_e2e.py` : **23/23** après correction du second consommateur de diagnostics, `run_forgejo_live_pr_merge_e2e.py`, qui exige lui aussi la phase `connect` avant de classer les trois routes comme bloquées.
- `python3 scripts/run_mission_012_stdio_mcp_egress_gate_e2e.py --forgejo-binary /tmp/forgejo-17.0.0 --state-dir /tmp/forgejo-agent-egress-stdio-mcp-010 --evidence-dir evidence/continuation-311-route-probe-connect-phase-001` : `verdict=passed`; reçu et provenance complets dans `result.json` et `provenance.json`.

Le premier lancement de la suite complète `test_forgejo_live_pr_merge_e2e.py` a exposé une comparaison de dictionnaire exacte qui ne connaissait pas le nouveau champ de diagnostic dans `test_supervisor_observes_sandboxed_agent_namespaces_from_proc`. Après mise à jour du consommateur `run_forgejo_live_pr_merge_e2e.py` et de son validateur de phase, la suite complète passe 23/23. Cette modification du consommateur distinct est postérieure au parcours composé 311; elle n’est pas attribuée à son exécution. Le parcours 311 n’a pas été rejoué; son `runner_sha256` reste lié à la version réellement utilisée dans `provenance.json`, et le test complet couvre le consommateur distinct sur la source courante.

Le run démarre une instance jetable Forgejo locale `17.0.0+gitea-1.22.0`, binaire SHA-256 `e5ec437cd6357e8d1b4580cb47ab1daa58a32b33971a63db63446a535d33adb7`; le commit de build du binaire est inconnu. Les contrôles hôte voient health, dépôt API et dépôt Web en HTTP 200. Les sondes hôte pour les chemins PR donnent 404 (aucune PR fixture), et receive-pack 401/403. Dans un processus enfant réel sous Bubblewrap `--unshare-net --unshare-pid`, les trois `connect()` vers `127.0.0.1:3130` échouent avant `sendall`, chacun avec `errno=111`; les noms des routes sont liés à leurs chemins exacts dans le reçu. Les contrôles positifs confirment que le listener Forgejo existe pendant l’expérience.

Au même instant, le parent fiable observe le même processus enfant vivant avant/après et lit `main` par GET API HTTP 200. Le SHA de la branche reste `d4520b86d2518523f3799735217e10fc60655ae7`. Un credential gate synthétique à portée `write:repository` reste dans le parent/memfd `CLOEXEC`, non transmis, non sérialisé et illisible depuis le namespace PID enfant. Le compteur `supervisor_observed_route_connections` est limité au listener MCP éphémère; ce n’est pas une trace de sockets Forgejo.

Le task adapter exécute un vrai test Rust hors ligne dans la fixture : l’acceptation échoue d’abord (exit 101, assertion `left: -2`, `right: 10`), le changement de source passe par MCP, puis le test protégé passe (exit 0, un test réussi). Forgejo se termine avec code 0 et le listener 3130 est absent. Le run n’emploie aucun modèle ou service externe.

## Portée et limites

Cette preuve renforce un parcours existant, elle n’en constitue pas un nouveau parcours de livraison. Le `local-test-adapter` est déterministe, `n=1`; aucun LLM/AgentOS, aucune PR ou requête merge, aucune fusion contrôlée ni comparaison de productivité. Les errno sont auto-déclarés par le fixture enfant; le parent vérifie l’identité des namespaces et les contrôles positifs, mais ne trace pas les syscalls du noyau. Les trois probes visent le même listener TCP et ne prouvent pas l’absence de tout egress. Forgejo et le runner partagent l’UID local; aucune propriété multi-tenant ou résistance à un noyau compromis n’est établie. Le binaire Forgejo affiche la version mais son commit de build reste inconnu.

Commandes, versions, empreintes et sorties : fichiers adjacents, `result.json`, `provenance.json`, `acceptance-baseline.log` et `acceptance-corrected.log`. Pas de test Rust produit relancé, pas de changement de production, publication, dépense modèle, secret externe ou choix de licence.
