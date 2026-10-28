# MISSION-012-LIVE-AGENT-WEBHOOK-EGRESS-003 — agent isolé, gate Forgejo et comportement post-fusion

Date : 26 octobre 2026. Résultat : **parcours réussi dans une instance Forgejo jetable locale, n=1, avec `local-test-adapter`**. Ce run n'a appelé aucun LLM.

## Environnement et commande

Le worktree produit était au HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. Le binaire Forgejo `/tmp/forgejo-17.0.0` annonce `17.0.0+gitea-1.22.0`, SHA-256 `e5ec437cd6357e8d1b4580cb47ab1daa58a32b33971a63db63446a535d33adb7`; son commit de compilation exact n'est pas établi. La référence source examinée est Forgejo v17.0.0, commit `0d791421f85ca13a63b665053d81ea765fbbc571`.

Runtime : Python 3.12.3, Bubblewrap 0.9.0, Cargo 1.95.0 et rustc 1.95.0, Linux aarch64. Commande exécutée :

```sh
python3 scripts/run_mission_012_live_composed_e2e.py \
  --state-dir /tmp/mission-012-live-agent-webhook-egress-003 \
  --forgejo-binary /tmp/forgejo-17.0.0 \
  --evidence-dir "$PWD/evidence/mission-012-live-agent-webhook-egress-003" \
  --execution-id live-mcp-100 \
  --allocation-id mission012-egress-100 \
  --agent-adapter local-test-adapter \
  --sandbox-test-adapter
```

Le reçu `composed-run.json` indique `merged_in_private_sandbox`, `model_invoked_during_run=false`, coût modèle rapporté 0, aucun jeton de modèle et une durée agent de 1 033 ms. La composition inclut l'admission par livraison de webhook, la demande et ses exigences (SHA-256 `3a3c444075c93a3bcfb2bec17d1a2e74ec0051effc2994e6b02ac095eb0dd431`), le contexte MCP, la modification autorisée de `src/lib.rs`, la PR Forgejo #2, la revue, la vérification indépendante, le gate et la réconciliation des notifications de PR.

## Egress et autorité du gate

Pendant l'appel MCP, le vrai processus `mission_012_test_agent_adapter.py --stdio-mcp` a été lancé dans Bubblewrap avec namespaces réseau et PID. Les reçus du bac à sable indiquent des namespaces différents de ceux de l'hôte, les descripteurs `0,1,2`, une allowlist d'environnement sans credential, `credential_visible=false`, `network_connected=false` et zéro connexion de route observée par le superviseur. Les trois sondes vers le listener Forgejo réel `127.0.0.1:3132` sont refusées depuis l'agent :

- API : `GET /api/v1/repos/forge-admin/mission-012-clamp/pulls/2` ;
- Web : `GET /forge-admin/mission-012-clamp/pulls/2` ;
- Git receive-pack : `POST /forge-admin/mission-012-clamp.git/git-receive-pack`.

Les opérations du gate hôte, distinctes du processus agent, ont fonctionné : Forgejo a accepté la lecture de la PR et le merge autorisé a retourné HTTP 200. Une fusion avant revue a été refusée HTTP 405; une tentative de fusion par le compte de revue non autorisé a aussi été refusée HTTP 405. La preuve de merge lie la base `847bc304522b87ba99c7f0495077a9954099fbd9`, la tête candidate `912acf8383344ecc2f0c406ec8ce86b1c3a3af4b` et le commit de fusion `8a2fcd8a60f759fb139cfc4dbf6870b959dd2691`. Le gate a rapporté `external_writer_exclusion=not_provided_by_cooperative_lock` : ce run n'établit pas une exclusion serveur des autres writers Forgejo.

## Échec actionnable, vérification et fusion

Le test protégé sur la base inchangée échoue de façon exploitable : exit 101, valeur obtenue `-2` au lieu de `10`, avec un test accepté et un échoué. L'adaptateur local a lu le contexte, effectué l'écriture MCP sur le seul chemin autorisé, puis obtenu deux tests réussis et aucun échec. La PR ne change que `src/lib.rs`. Le vérificateur indépendant a lié ses résultats aux commits et aux empreintes des exigences, de la politique, du candidat et du fichier d'acceptation; il a refait la vérification immédiatement avant la décision du gate.

Après le merge, un clone local détaché de `refs/heads/main` relit le commit de fusion exact. L'arbre du merge égale l'arbre candidat, le diff depuis la base ne contient que `src/lib.rs`, et le SHA-256 du source lu depuis `main` égale celui du candidat approuvé (`173304ad7d32aabc1e8e1ee41eabda3398a8690798e68f906bcebc73cbcd6d15`). `cargo test --offline --locked` sur ce checkout post-fusion passe avec `2 passed; 0 failed` (`postmerge-readback-retry-002.json`, journal de commande associé). Cela vérifie le comportement du code fusionné dans cette fixture locale.

Le webhook PR a livré trois événements (ouverture, approbation, fermeture), reçus HTTP 202 et réconciliés en relisant l'état Forgejo courant. Le hook a ensuite été désactivé et relu inactif; le récepteur a été arrêté et son port relu sans écouteur.

## Incident du harnais et limites

Deux essais précédents du test egress composé, conservés dans leurs états privés sous `/tmp`, ont échoué avant le stade PR. Ils ont révélé que le fixture de l'adaptateur attendait un autre source (`clamp_inclusive`, variable `value`) que la source réelle épinglée (`clamp_to_range`, variable fautive `minimum`). Un test unitaire reproduit ce refus, puis passe après alignement du fixture sur la source et la correction attendue (`minimum` → `maximum`). Les suites ciblées couvrent l'adaptateur, son wrapper Bubblewrap et les arguments du runner.

Vérifications ciblées après correction : `python3 -m unittest tests.test_mission_012_live_agent_pr_e2e tests.test_mission_012_live_mcp_agent_e2e tests.test_mission_012_sandboxed_test_adapter tests.test_mission_012_test_agent_adapter -v` passe `55/55`; `python3 -m unittest tests.test_forgejo_emitted_webhook_e2e -v` passe `92/92`. Les sorties sont conservées dans ce dossier. Le run composé ci-dessus constitue la vérification d'intégration au-delà de ces tests de contrat.

Le premier essai du test post-fusion a choisi le shim `/home/pc6/.cargo/bin/rustup` comme binaire `cargo`, qui a refusé `--offline`. Ce résultat est conservé dans `postmerge-cargo-test.log`; la reprise sur un nouveau clone utilise le binaire Cargo du toolchain 1.95.0 et passe. Ce premier résultat est une erreur de commande du harnais, pas un échec du code fusionné.

La frontière prouvée dépend de Bubblewrap et de la configuration de cet hôte : même UID entre le serveur et le runner, pas de résistance démontrée à une évasion noyau ou à un acteur privilégié, et seules ces trois routes ont été sondées. Le binaire Forgejo n'est pas relié de façon attestée au commit source examiné. Le `local-test-adapter` n'est ni AgentOS ni un LLM. `n=1` ne mesure aucun gain comparatif, coût de modèle, temps humain ou supériorité par rapport à Forgejo avec agent et CI conventionnels. Aucun code Forgejo/GitSpace n'a été copié ou adopté; aucune production, publication ou dépense modèle n'a été engagée. Le projet reste incomplet.
