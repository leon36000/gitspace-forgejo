# CONTINUATION-180 — frontière egress de l’adaptateur vivant pendant un merge Forgejo

Run local le 27 octobre 2026 dans le worktree `webhook-005-workspace`, base `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`, fortement modifié. Aucun commit n’a été créé. Forgejo a été lancé uniquement par le runner local avec une base jetable sous `/tmp`; aucun service de production ou service externe n’a été utilisé.

## Défaut du runner trouvé avant la preuve

Les tentatives 001–009 du lot n’ont pas fourni de reçu complet valable. La dernière montrait un résultat réussi de l’adaptateur et la fermeture MCP `shutdown`, mais `run_task_at_barrier` comparait la séquence à une liste qui omettait `shutdown`; le validateur rejetait donc après la fin du protocole. Le test du nouveau contrat a d’abord échoué sur l’absence du helper `expected_mcp_methods`, puis a passé après l’ajout de l’arrêt attendu dans la séquence. Le support de réponse MCP `shutdown` et son test séparé sont dans le code courant. Aucun résultat 009 n’est compté comme parcours réussi.

## Commande, sources et environnement

Commande exacte enregistrée dans `command.txt` :

```text
python3 scripts/run_forgejo_live_pr_merge_e2e.py --forgejo-binary /tmp/forgejo-17.0.0 --repo-root /home/pc6/Documents/Codex/2026-10-21-mission-construire-une-forge-con-ue/.runtime/webhook-005-workspace --state-dir /tmp/forgejo-agent-cont180-state-010 --non-admin-merger --stdio-mcp-egress-during-merge
```

Le binaire annonce `17.0.0+gitea-1.22.0` et son SHA-256 est `e5ec437cd6357e8d1b4580cb47ab1daa58a32b33971a63db63446a535d33adb7`. Le commit source exact de ce binaire local n’est pas établi; le présent test ne valide donc pas un commit Forgejo précis. Forgejo/GitSpace n’ont pas été copiés ni modifiés. Les versions/outils sont dans `environment.txt`; les empreintes des sources examinées sont dans `source-hashes.txt`.

Le nouveau test `test_expected_mcp_protocol_includes_shutdown_after_task_calls` lie maintenant les deux séquences attendues (avec et sans barrière) à `shutdown`. Le runner garde le receipt du traitement, et le host MCP répond au shutdown puis quitte. La suite complète pertinente passe : runner PR 15/15, host MCP 2/2, wrapper sandbox 8/8 et runner d’egress 9/9. Compilation Python et `git diff --check` passent également. Les sorties capturées figurent dans les fichiers `*.log` et `verification-results.json`.

## Observation Forgejo et tâche exécutée

Le run crée une issue locale et une PR `#2` sur la branche `main`. La tâche est un défaut logiciel réel dans la petite fixture : `clamp_inclusive` ne borne pas les valeurs au maximum. La mission MCP locale reçoit le contexte de tâche, lit le code, exécute `cargo test`, observe le rouge puis remplace l’expression fautive. Le candidat lié à la PR est le commit `365fcc7924292c18cedd58db3cc7e4e9ce5ff60f`; le SHA-256 du source est `6937c269b5a838c3eb28343721f59774b3d9a15bac9da5016d788fa8e0005c04`.

L’acceptance initiale échoue avec code 101 sur les cinq cas de borne (valeurs basse/haute, intervalle singleton et extrêmes `i32`), tandis que quatre cas passent. Après correction, l’acceptance protégée passe 9/9. Le processus sandboxé qui a produit la modification reste vivant sur la barrière après acceptation. Le gate hôte relit la PR et la base `main` (`58759efa8ee4fdb958a46124d41d5caf6ec6bd1f`), consomme sa capacité de test liée à la cible, vérifie de nouveau avant l’effet, fusionne en HTTP 200, puis relit la tête `main`. La tête relue et le commit de fusion sont tous deux `29db6077dfe4f28e23941b17e96a53e6364b7786`; après le merge le même processus MCP est libéré, finit ses appels et effectue `shutdown`. Une exécution d’acceptance depuis le code post-fusion passe 9/9 et son hash source correspond au hash du candidat fusionné. Les reçus expurgés se trouvent dans `forgejo-run-summary.json`, `mcp-egress-gate-summary.json` et `gate-admission-summary.json`.

## Frontière observée et portée

Le processus local de test-adapter est lancé sous Bubblewrap avec `--unshare-net` et `--unshare-pid`, `/tmp` et `/home` en tmpfs, `/usr` en lecture seule, environnement vidé/reconstruit et seuls les descripteurs 0, 1 et 2 ouverts. Il rapporte un namespace réseau et PID différents de l’hôte. Ses connexions directes vers l’API, la Web UI et `git-receive-pack` de l’instance locale échouent; la tentative de lecture du credential gate via le chemin hôte `/proc/<pid>/fd/<fd>` échoue également. Le credential reste dans le processus hôte de confiance puis dans le descripteur reçu par le binaire Rust du gate; il n’est pas passé au processus de tâche ni persisté en fichier. En parallèle, le gate réussit sa lecture authentifiée de `main` puis le POST de merge. L’émission de capacité lie cette cible locale et précède l’exécution du gate; le JSON indique toutefois explicitement `supervisor_policy_independently_verified=false`.

Précision sur la preuve : le champ historique `supervisor_observed_route_connections=0` compte un listener loopback éphémère du wrapper, pas une trace indépendante des connexions visant le port Forgejo 3130. Il n’est pas pris comme preuve principale. La preuve retenue ici est la commande Bubblewrap courante, les namespaces distincts, les échecs de connexion observés depuis le processus de tâche, l’absence du credential et le succès concomitant du gate. Cela ne constitue pas une politique réseau système générale ni une observation indépendante d’un agent arbitraire.

## Limites

- L’exécutant est le `local-test-adapter` déterministe; aucun LLM, AgentOS ou runtime de production n’a été appelé. Le passage ne valide pas le comportement d’un agent LLM hostile ni une intégration AgentOS réelle.
- La tâche, le dépôt, Forgejo et les comptes sont jetables et locaux (`n=1`). Acteurs privilégiés, processus du même UID exécutés hors sandbox, noyau compromis, évasion de Bubblewrap et autres writers Forgejo ne sont pas couverts.
- La réussite du gate ne démontre pas que sa politique superviseur est vraie indépendamment; le reçu conserve `supervisor_policy_independently_verified=false`. La capacité locale ne prouve pas la correction de la modification.
- Trois routes précises sont sondées dans ce scénario; ce n’est ni une revue exhaustive des protocoles réseau ni une certification de sécurité.
- Ce test n’ajoute aucun résultat de comparaison Forgejo + agent + CI conventionnels, aucune mesure de tokens/coût et aucun gain statistique. Licence GitSpace/Cargo et droits de redistribution restent ouverts.

La mission globale demeure incomplète. Le run établit seulement la frontière locale du test-adapter pendant une fusion et la correction du validateur de fermeture MCP.
