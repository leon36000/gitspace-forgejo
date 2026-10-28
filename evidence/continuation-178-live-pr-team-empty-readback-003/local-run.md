# CONTINUATION-178 — readback live d’équipes reviewers vides et correction du traceur

Run local le 27 octobre 2026 dans le worktree `webhook-005-workspace`, HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`, checkout fortement modifié (1 522 entrées `git status --porcelain` au relevé). Aucun commit n’a été créé. Le test Forgejo est une instance jetable locale; aucun accès, secret utilisateur ou service externe n’est employé.

## Défaut du harness et correction

Le run 002 a reçu les huit webhooks (tous HTTP 202), redémarré le receiver après SIGKILL et confirmé le doublon idempotent, puis échoué au stade `reconcile_issue_and_pull_request_events`. Le trace contient deux appels `connect`, tous deux à `127.0.0.1:3130`; le parseur de l’observateur réclamait exactement un appel. Le readback de la réconciliation n’a pas été enregistré, car la garde échouait avant la validation des résultats. La copie du `failure.json`, du trace, des readbacks partiels et de leur checksum est sous `prior-attempts/attempt-002/`; les originaux restent dans `evidence/continuation-178-live-pr-team-empty-readback-002/`. Le run 001 s’est arrêté au garde qui refuse de réutiliser un dossier de preuve existant, avant de démarrer Forgejo; sa console est dans `prior-attempts/attempt-001/`.

Le test ajouté `ParseLoopbackConnectTraceTests.test_accepts_multiple_connections_to_the_configured_fixture` a d’abord échoué contre le parseur antérieur avec `ValueError: batch reconciliation did not use exactly one loopback connection`. Le changement minimal accepte désormais toute séquence non vide de connexions vers l’hôte/port loopback configurés, conserve chaque code de résultat et garde les refus pour destination externe, forme inconnue et trace vide. Le champ JSON singulier `connect_result` reste émis quand il n’y a qu’un appel. Les cinq tests du parseur passent; la suite du fichier `tests/test_forgejo_emitted_webhook_e2e.py` passe 128/128. Le runner modifié est `scripts/run_forgejo_live_webhook_reconcile_emitted.py`; aucun code Rust/Cargo n’a changé dans cette tranche.

## Commande et instance réussies

Commande exacte :

```text
python3 scripts/run_forgejo_live_webhook_reconcile_emitted.py --forgejo-binary /tmp/forgejo-17.0.0 --repo-root /home/pc6/Documents/Codex/2026-10-21-mission-construire-une-forge-con-ue/.runtime/webhook-005-workspace --state-dir /tmp/forgejo-agent-cont178-state-003 --evidence-dir /home/pc6/Documents/Codex/2026-10-21-mission-construire-une-forge-con-ue/.runtime/webhook-005-workspace/evidence/continuation-178-live-pr-team-empty-readback-003 --request-review --batch-pull-request-events --observe-batch-network
```

Forgejo rapporte `17.0.0+gitea-1.22.0`, Go 1.26.9 dans sa chaîne de build, SHA-256 binaire `e5ec437cd6357e8d1b4580cb47ab1daa58a32b33971a63db63446a535d33adb7`; le commit exact du binaire n’est pas établi. L’hôte est Linux aarch64, Python 3.12.3, Rust/Cargo 1.95.0 et strace 6.8. Les versions et le binaire sont dans `runner-console.log` et `environment.txt`.

Observations Forgejo et receiver :

- La création de demande de review à l’utilisateur local `forge-reader` répond HTTP 201; le readback de son rôle indique `repo_permission_pull=true`, `repo_permission_push=false`, `repo_permission_admin=false`.
- Forgejo émet huit tâches hook; le readback de chacune confirme livraison réussie et HTTP 202. Les événements incluent l’ouverture/fermeture PR, commentaire de conversation et demande de review.
- Les GET Forgejo live et les résultats du CLI projettent `requested_reviewers_teams=[]`. C’est une liste vide courante pour ce dépôt appartenant à un utilisateur; ce n’est pas un test de demande d’équipe non vide ou d’accès à la route d’organisation. La PR termine fermée sans merge.
- Le trace strace du seul batch de réconciliation contient deux connexions, toutes deux `127.0.0.1:3130`; aucun argv, header ou corps de requête n’est tracé. Le JSON brut, le parseur de trace et les réconciliations sont conservés dans ce dossier.
- Le receiver est tué par SIGKILL puis relancé sur le même journal. La livraison dupliquée retourne 202 avec `duplicate=true`; octets, nombre d’enregistrements et SHA-256 du journal restent identiques avant/après.
- Le hook est désactivé avant l’arrêt de Forgejo. Receiver et Forgejo terminent; aucun listener n’est observé après arrêt et `cleanup_failure=null`.

## Vérifications et artefacts

- `python3 -m unittest discover -s tests -p 'test_forgejo_emitted_webhook_e2e.py' -v` : 128 tests réussis.
- `python3 -m py_compile scripts/run_forgejo_live_webhook_reconcile_emitted.py tests/test_forgejo_emitted_webhook_e2e.py` : réussi.
- `artifact-validation.json` revalide indépendamment la sortie structurée, les huit acquittements, les deux destinations loopback, les quatre résultats de batch et l’absence de changement au journal lors du replay.
- L’exécution live retourne `verdict=passed`; ses résultats structurés, tâches Forgejo, trace, replay et cleanup sont inclus dans le reçu.
- Le test seul prouve le comportement de l’adaptateur local et de Forgejo v17.0.0 sur les routes exercées. Ce n’est ni une vérification indépendante du logiciel modifié, ni une fusion, ni le parcours avec LLM/AgentOS, ni un test de frontière egress pour le processus agent.

## Limites et état conservé

L’observation réseau porte sur les appels `connect` TCP du seul processus CLI suivi; elle ne définit pas une politique d’egress système et ne détecte pas les sockets héritées. Les reviewers équipes non vides ne sont pas testés. Le run ne fusionne pas la PR, n’utilise pas un modèle, ne mesure ni tokens ni coût, et n’établit aucun gain comparatif. Les fichiers/code Forgejo ne sont pas copiés au produit. Le projet Cargo et la licence GitSpace restent sans licence sélectionnée; droits de redistribution non évalués. L’état privé synthétique du run est conservé sous `/tmp/forgejo-agent-cont178-state-003` et n’est pas inclus dans ce reçu. Le projet demeure incomplet.
