# CONTINUATION-358 — milestone courant transmis au contexte d’admission

Date : 28 octobre 2026. Dépôt isolé : `local/WEBHOOK-005-canonical-links`, HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. Le worktree était déjà fortement modifié et contenait des fichiers non suivis. Aucun fichier n’a été supprimé, écrasé par reset, stagé, commité ou publié. Aucun secret n’a été lu ou copié.

## Changement

Le reconciler pouvait exposer le milestone courant, mais le chemin d’admission vers l’agent ne le transportait pas. `forgejo-mission-local` projette maintenant le milestone de la même lecture API que le reste de l’issue vers `untrusted_work_item`. Les valeurs du webhook ne sont pas utilisées.

Le contrat imbriqué passe de work-item v2 à v3 et la requête agent de v3 à v4 lorsque ce contexte est présent. Un enum sérialisé distingue l’absence de champ API (`unknown`), le `null` explicite (`unassigned`) et l’objet attribué (`assigned`). L’objet est réduit à ID, titre et état; l’ID doit être positif, le titre non vide, sans caractères de contrôle et limité à 1 024 octets, et l’état doit être `open` ou `closed`. Une valeur invalide refuse l’admission, même si le corps facultatif de l’issue est absent. Les schémas antérieurs gardent leur empreinte et leur validation historiques.

Le SHA structuré du contexte v3 lie le milestone et les labels connus. La validation de la requête vérifie le couple requête v4/contexte v3. Le host MCP inclut désormais le milestone dans la frontière de confiance et le signale comme entrée utilisateur non fiable. Les exigences JSON et leur SHA restent dans des champs séparés. Le test-adapter local valide le nouveau schéma et ses bornes; le contenu hostile ne change ni le nombre de tests, ni la correction ni le texte de résultat.

## Reproduction et correction

Le test unitaire ajouté en premier a échoué parce que le JSON admis ne contenait pas `milestone`. Au premier run composé, l’API simulée produisait `unknown`; l’examen du chemin a montré que `mission_012_fixture_response` ne copiait que titre, corps et labels. La fixture relaie maintenant `milestone` lorsqu’il existe et préserve le champ absent autrement. Un test rouge du test-adapter a montré qu’il rejetait toutes les versions autres que 1/2; il accepte maintenant la version 3 avec validation stricte de l’observation, de l’ID, du titre et de l’état. Le test malformé échoue avant `cargo_test` ou écriture.

## Vérifications

- `cargo test --offline --locked --all-targets -- --test-threads=1` : 38 groupes, 739 réussis, 0 échec, 166 ignorés. Journal : [all-targets.log](all-targets.log). Les scénarios annotés `ignored` n’entrent pas dans ce total.
- `python3 -m unittest tests.test_mission_012_test_agent_adapter` : 13/13 réussis; le cas assigné avec titre hostile est observé mais non utilisé et l’ID nul est refusé avant test/écriture. Journal : [python-adapter.log](python-adapter.log).
- `python3 scripts/run_forgejo_mission_delivery_process_e2e.py --only actual_cli_current_issue_labels_reach_mcp_context_from_signed_webhook` : 1/1. Le vrai receiver signe et journalise un webhook synthétique contenant un milestone périmé ID 99; le vrai CLI lit une API HTTP loopback avec milestone courant ID 12; `mission_context` reçoit le snapshot courant, les exigences restent inchangées et le texte de frontière marque aussi le milestone comme non fiable. L’arrêt sans preuve indépendante est refusé. La source est montée en lecture seule dans Bubblewrap avec namespaces réseau et PID séparés. Journal : [context-e2e.log](context-e2e.log).
- `cargo test --offline --locked --test mission_012_mcp_lifecycle -- --test-threads=1` : 30 réussis, 1 ignoré; couvre notamment le texte de frontière mis à jour.
- `cargo fmt --all -- --check`, `python3 -m py_compile` et `git diff --check` : code retour 0; journaux [rustfmt.log](rustfmt.log), [python-compile.log](python-compile.log), [git-diff-check.log](git-diff-check.log).

## Portée et limites

Le test processus prouve un transport avec la vraie CLI, le vrai receiver et le vrai host MCP, mais son API est une fixture loopback, non Forgejo vivant. La validation du test-adapter Python utilise un client MCP fictif et ne prouve pas l’usage d’un modèle. Aucun AgentOS ou LLM n’a été appelé; aucune PR, fusion, mesure de tokens/coût ou comparaison de productivité n’est établie. Une valeur du milestone reste descriptive, non une exigence, preuve ou autorisation. La lecture n’est pas atomique avec le début futur d’une exécution; le host conserve la détection de conflit lorsque les octets de requête changent sous la même clé d’idempotence.

Pas de nouvelle dépendance, pas de copie du code Forgejo et aucun changement à `Cargo.toml`/`Cargo.lock` dans ce lot. La licence du produit reste non choisie; les droits de redistribution ne sont pas évalués.

Le transfert GitLab demandé n’a pas été tenté dans ce lot. CONT-357 rapporte que le PAT reçu par courriel a déjà figuré dans une sortie du connecteur et que PC4 était injoignable (DNS MagicDNS et TCP/HTTPS). Ne pas réutiliser ce jeton; le révoquer/renouveler et rétablir le réseau avant de lire les refs GitLab.

Sources modifiées et empreintes : [source-hashes.txt](source-hashes.txt). Le manifeste local et les index append-only lient ce rapport, les journaux et les documents canoniques.
