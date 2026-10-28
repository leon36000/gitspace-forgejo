# FORGEJO-WEBHOOK-CREATE-ZERO-SHA-PROCESS-E2E-001

## Résultat

Le test `actual_receiver_rejects_zero_sha_branch_create_before_durable_append` lance le binaire receiver Rust réel sur loopback avec secret HMAC temporaire, envoie un événement `create` signé contenant un SHA de 40 zéros et observe HTTP 400 / `event_payload_invalid` / `retryable=false`. Le journal reste octet pour octet identique à son état vide; après arrêt du processus, son rejeu ne contient aucun événement. C'est une preuve de frontière processus pour le rejet introduit dans [le lot précédent](../forgejo-webhook-create-zero-sha-refusal-001/local-run.md), pas un nouveau changement de logique d'exécution.

Le test ciblé passe 1/1 en 0,06 s ([journal](receiver-process-focused.log)). La suite complète finale `cargo test --locked --offline --all-targets -- --test-threads=1` passe 549/549, 0 échec, 76 ignorés, 34 groupes ([journal de reprise](rust-all-targets-retry.log)). Format, diff-check et garde de métadonnées Cargo passent. L'état Cargo est encore `license=null`, `license_file=null`, `publish=[]`, `no_license_selected`, droits `not_assessed`.

## Échec de vérification conservé

Le premier run complet après ajout du test a échoué dans le test indépendant `mcp_supervisor_cancellation_terminates_the_in_flight_verifier_scope` à `tests/mission_012_mcp_lifecycle.rs:1728`, après avoir observé un PID dans le cgroup au terme de la grâce de deux secondes ([journal](rust-all-targets.log)). Un run isolé suivant a reproduit l'assertion une fois ([journal](cancellation-isolated.log)); immédiatement après le retour de ces échecs, le PID n'était plus présent dans `/proc`. Une assertion diagnostique du test collecte désormais, en cas de nouvelle observation, l'état, le groupe de processus, le cgroup et la ligne de commande du résidu. Le run isolé avec cette instrumentation passe 1/1 ([journal](cancellation-diagnostic.log)); trois répétitions isolées supplémentaires passent chacune 1/1 en 15,80-15,81 s ([1](cancellation-repeat-1.log), [2](cancellation-repeat-2.log), [3](cancellation-repeat-3.log)); la suite complète de reprise passe et exécute le nouveau test processus.

Ce motif intermittent existe dans des preuves antérieures; aucune cause précise n'est établie et le code d'exécution n'a pas été modifié ici. La réussite de reprise ne supprime pas l'échec précédent ni ne certifie la cancellation. Cette incertitude reste une lacune du projet à résoudre et à revalider, sans élargir ce lot de webhook.

## Portée et limites

Le nouveau cas vérifie l'application du parseur au processus HTTP receiver et l'absence d'effet durable après refus. Il utilise un inbox et un secret jetables; aucun serveur Forgejo réel n'a produit ce payload artificiel, aucun accès réseau externe ni modèle n'a été utilisé. Il ne démontre pas un parcours LLM/AgentOS, une comparaison de productivité, ou une sûreté générale des writers Forgejo. Le dossier conserve aussi les essais incomplets/échoués afin de ne pas présenter seulement le résultat positif.
