# WEBHOOK-PR-DELETED-RESOURCE-PROCESS-001 — refus d’une PR supprimée

Date : 25 octobre 2026. `HEAD` observé : `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`; le worktree est fortement modifié, donc le commit ne suffit pas à identifier les sources exécutées. Les empreintes du test, du runner et des documents sont dans `source-sha256.txt`.

## Comportement exercé

Le scénario `actual_reconcile_cli_process_reports_deleted_pull_request_in_single_delivery` traverse le receiver `forgejo-webhook-local`, son inbox durable, puis le processus `forgejo-reconcile-local`. Le test signe localement un événement synthétique `pull_request` pour la PR 23; le receiver répond `202`, l’événement est journalisé, puis le CLI interroge exactement `/api/v1/repos/forge-admin/agent-forge-core/pulls/23`.

La fixture HTTP loopback renvoie `404` et un corps marqueur non fiable. Le CLI sort avec le JSON structuré `schema_version=3`, `verdict=inconclusive`, `error.code=resource_not_found`, `retryable=false`, et `details.resource.kind=pull_request` pour le dépôt et le numéro attendus. Aucun `canonical_state` n’est émis, aucun en-tête Authorization n’est envoyé, le corps d’erreur amont n’apparaît pas dans la sortie, et l’inbox est relue byte à byte sans changement après le CLI. L’exécution ciblée finale a réussi `1/1`; 41 autres tests du binaire ont été filtrés.

Le scénario est ajouté à la liste blanche du runner afin d’être exécutable seul. Il couvre le chemin de suppression d’une PR; les tests existants vérifient séparément le 404 d’une issue, la continuation d’un lot après une issue supprimée, les routes PR normales et les cas de preuve de merge périmée. Cette tranche n’a pas modifié la logique de réconciliation; elle ajoute une assertion processus qui exposait une différence de ressource jusque-là non exercée au niveau CLI.

## Exécution, licence et limites

Commande et résultat synthétique exact : `commands.txt` et `test-result.txt`. Le runner a compilé une copie verrouillée hors ligne et exécuté le test sous Bubblewrap avec namespaces réseau/PID séparés, home temporaire et fixture loopback. L’hôte était Linux/aarch64, Rust/Cargo 1.95.0, Python 3.12.3, Bubblewrap 0.9.0.

Le garde Cargo et `cargo metadata` confirment `license=null`, `license_file=null`, `publish=[]`, `license_decision=no_license_selected` et `redistribution_rights=not_assessed`; aucune licence n’est sélectionnée et la publication reste désactivée. Aucun agent LLM, AgentOS, serveur Forgejo réel, service externe ou secret n’a été utilisé. La fixture n’établit pas la compatibilité de toutes les versions Forgejo ni le comportement d’un déploiement. Le manifeste local et les nouvelles entrées append-only du manifeste global couvrent les preuves de cette tranche.
