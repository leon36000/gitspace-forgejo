# WEBHOOK-RETRY-SCHEDULE-002 — délai batch associé à chaque ressource

Date : 24 octobre 2026. Dépôt produit local `local/WEBHOOK-005-canonical-links`, HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. Worktree fortement modifié et préservé. Aucun commit, push, publication, changement de production ou appel modèle.

## Question vérifiée

Après WEBHOOK-RETRY-SCHEDULE-001, le champ `error.details.retry_after_ms` existait dans la sortie du vrai processus CLI pour une réponse single. Le nouveau scénario vérifie sa conservation dans un batch où deux livraisons distinctes désignent la même issue, une autre issue reçoit un autre délai, et une troisième issue répond avec succès.

## Fixture et résultat

Le test lance le vrai binaire `forgejo-webhook-local` pour recevoir quatre requêtes synthétiques signées et journaliser les livraisons, puis le vrai binaire `forgejo-reconcile-local` en mode batch. Le serveur HTTP de test est une fixture loopback dans `bwrap --unshare-net`, pas Forgejo. Il répond successivement :

1. issue 41 : HTTP 429, `Retry-After: 3600`;
2. issue 42 : HTTP 503, `Retry-After: 7200`;
3. issue 43 : HTTP 200 avec l’état courant valide.

Les deux IDs différents de l’issue 41 reçoivent chacun un résultat inconclusif lié au délai de **3 600 000 ms**, sans seconde lecture API. L’issue 42 reçoit le délai propre de **7 200 000 ms**. L’issue 43 conserve son résultat `current_state_read`; le batch reste globalement inconclusif à cause des deux erreurs. Il y a exactement trois GET pour quatre livraisons. Aucun en-tête Authorization n’est émis et les corps d’erreur de la fixture ne sont pas copiés dans la sortie.

La première exécution de cette tranche a révélé un défaut dans le nouveau helper de test : à la fin de sa fenêtre d’observation, il panique au lieu de retourner ses requêtes déjà reçues. Aucun résultat produit n’est conclu de cet essai. L’échec est conservé dans [process-e2e-final.log](process-e2e-final.log); après correction du helper, l’exécution finale du runner passe **13/13** scénarios dans [process-e2e-green.log](process-e2e-green.log). L’ancien [process-e2e.log](process-e2e.log) est conservé comme preuve du run antérieur et n’est pas utilisé pour conclure sur la version finale du scénario.

## Commandes et environnement

- `cargo fmt --all -- --check` — réussite.
- `python3 -m py_compile scripts/run_webhook_reconcile_cli_e2e.py` — réussite.
- `git diff --check` — réussite avant l’exécution du runner.
- `python3 scripts/run_webhook_reconcile_cli_e2e.py` — **13/13** cas réussis; le script construit le test d’intégration en cible temporaire et exécute chaque cas isolément sous Bubblewrap avec réseau désactivé sauf son loopback privé.

Versions : Rust/Cargo 1.95.0, Python 3.12.3, Bubblewrap 0.9.0. Source du test : [webhook_receiver_process_e2e.rs](../../tests/webhook_receiver_process_e2e.rs); sélection de cas : [run_webhook_reconcile_cli_e2e.py](../../scripts/run_webhook_reconcile_cli_e2e.py).

## Limites

Ce test vérifie le câblage batch/cache de la réconciliation locale contre des réponses HTTP synthétiques. Il ne contacte pas Forgejo, ne crée pas d’issue ou de PR, ne fait pas de fusion et n’emploie ni LLM, ni AgentOS, ni RAG, ni mémoire. Il ne démontre pas le comportement d’un ordonnanceur externe, un gain de productivité, une baisse de coût ou une correction générale. La comparaison Forgejo+agent+CI conventionnels n’est pas faite.
