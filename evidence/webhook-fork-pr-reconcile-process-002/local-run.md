# WEBHOOK-FORK-PR-RECONCILE-PROCESS-002

Date : 25 octobre 2026. Worktree observé au HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`, branche `local/WEBHOOK-005-canonical-links`, fortement modifié.

## Résultat

Le scénario `actual_reconcile_cli_process_refuses_foreign_current_base_repository_after_fork_delivery` complète le chemin de PR fork au niveau processus et vérifie le refus de l’identité de base au moment de lire l’état courant.

1. Le vrai `forgejo-webhook-local` reçoit une livraison HMAC signée `pull_request/synchronize`. Le dépôt racine et `pull_request.base.repo` désignent `forge-admin/agent-forge-core`; `pull_request.head.repo` désigne `contributor/agent-forge-core`. Le receiver retourne HTTP 202, `accepted=true`, `duplicate=false`, `reconcile_required=true`. Après arrêt du receiver, l’inbox contient exactement un événement et ses octets sont conservés comme référence.
2. La fixture API répond HTTP 200 pour le PR #44. Le numéro, le chemin/URL canonique, l’état, les SHA base/head et l’identité du fork head sont valides; seul `base.repo` désigne `other-owner/other-repository`.
3. Le vrai `forgejo-reconcile-local` exécute une seule lecture `GET /api/v1/repos/forge-admin/agent-forge-core/pulls/44`, sans header `Authorization`.
4. Le processus termine code 1 et renvoie `verdict=inconclusive`, `error.code=resource_mismatch`, `retryable=false`, sans `canonical_state`. Les octets de l’inbox après décision sont identiques à ceux pris avant.

Le test ignoré passe 1/1 sous Bubblewrap 0.9.0 avec namespaces réseau et PID isolés; 70 autres tests du binaire sont filtrés. Le runner compile le test et les binaires Rust hors réseau, depuis le code courant et le lockfile. Le contrat `pull_request_current_base_repository_is_bound_while_foreign_head_is_allowed` passe 1/1. `rustfmt --check`, `py_compile`, `git diff --check` et les sept tests du garde Cargo passent. Le résultat processus brut et les autres sorties sont archivés dans ce dossier.

## Vérification

Aucun changement runtime n’a été nécessaire : cette tranche ajoute le scénario processus et son entrée dans le runner. La suite `--all-targets` n’a pas été relancée; le dernier résultat complet (532 réussis, 68 ignorés, 34 groupes) précède ce test. Le scénario dédié est exécuté explicitement malgré son attribut `#[ignore]`.

## Reproductibilité

- Test : `tests/webhook_receiver_process_e2e.rs::actual_reconcile_cli_process_refuses_foreign_current_base_repository_after_fork_delivery`.
- Runner : `scripts/run_webhook_reconcile_cli_e2e.py`.
- Commandes : `commands.txt`.
- Versions : `toolchain.txt`.
- Hashes des sources et du lockfile : `source-sha256.txt`.
- Résultat brut : `process-e2e.log`.
- Manifeste SHA-256 : `SHA256SUMS`.

## Portée et limites

C’est un test de processus local avec HMAC et HTTP loopback de fixture, pas un PR réel dans Forgejo. Il prouve que le receiver accepte un événement de fork ciblant le dépôt local configuré, puis que la réponse API qui déplace `base.repo` vers un autre dépôt ne devient pas canonique. Il ne prouve pas les ACL, la forme complète de tous les payloads Forgejo, les permissions de merge, ni une fusion.

Aucun LLM, AgentOS, Forge Code, RAG, moteur de mémoire, credential, service externe, coût ou donnée privée n’a été utilisé. Cargo reste `license=null`, `license_file=null`, `publish=[]`, décision `no_license_selected`, droits de redistribution `not_assessed`; aucun droit n’est inféré. Aucun gain comparatif n’est démontré et la mission reste incomplète.

Suite utile : vérifier avec un banc Forgejo local isolé si un PR de fork natif produit la forme de base/tête utilisée par ce contrat, sans démarrer, arrêter ou modifier un service inconnu.
