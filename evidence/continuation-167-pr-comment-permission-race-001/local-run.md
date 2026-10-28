# CONTINUATION-167 — Relecture du parent après un 404 de commentaire

Date : 26 octobre 2026, heure locale (27 octobre UTC). Ce lot s’est déroulé dans le worktree local isolé. Le dépôt était déjà fortement modifié. Le commit examiné est `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`; aucun commit n’a été créé.

## Constat et modification

Un commentaire d’issue ou de PR supprimé peut répondre HTTP 404 parce qu’il n’existe plus, ou parce que l’accès au dépôt a été révoqué après la lecture de son parent. Déclarer le commentaire absent à partir du succès parent antérieur peut donc projeter un état après une perte de visibilité. Le réconciliateur relit le parent après le 404 du commentaire `deleted`. En batch, cette lecture évince/remplace l’entrée parent mise en cache; si la lecture fraîche échoue, le résultat reste inconclusif et ne contient aucun état canonique.

Un test de contrat remplit le budget JSON du batch entre la lecture du parent/commentaire et l’événement `deleted`. La relecture refusée par le budget ne réutilise pas le parent en cache. Un autre test change l’état, le head, la base et le commit de merge entre les lectures, puis vérifie que la réconciliation rend les champs du parent fraîchement lu. Le défaut d’interface est documenté : l’implémentation par défaut de `read_current_fresh` convient aux lecteurs sans cache; un adaptateur qui cache `read_current` doit évincer ou contourner son propre cache.

Les validateurs Python exigent maintenant trois lectures amont pour un commentaire `deleted` dont le GET direct répond 404. Les commentaires présents et les événements `created` périmés restent à deux lectures.

## Vérifications

Commandes exécutées sur le worktree courant :

- `cargo test --offline --locked --test webhook_reconcile_contract` — 64 réussis, 0 échec.
- `cargo test --offline --locked --test webhook_reconcile_contract batch_budget_exhaustion_cannot_reuse_cached_parent_for_deleted_comment_404 -- --exact --nocapture` — 1 réussi, 0 échec.
- `python3 -m unittest tests.test_forgejo_emitted_webhook_e2e` — 120 réussis, 0 échec.
- `python3 scripts/run_webhook_reconcile_cli_e2e.py --scenario actual_reconcile_cli_process_shares_current_comment_read_across_create_delete_batch` — 1/1; le scénario couvre deux ordres d’événements, chacun effectuant les lectures parent/commentaire/parent frais.
- `python3 scripts/run_webhook_reconcile_cli_e2e.py --scenario actual_reconcile_cli_process_reuses_pull_request_parent_and_comment_read_across_batch_orders` — 1/1; deux permutations sont exercées.
- `python3 scripts/run_webhook_reconcile_cli_e2e.py --scenario actual_reconcile_cli_process_keeps_deleted_pr_comment_inconclusive_when_parent_access_is_revoked_between_reads` — 1/1; la séquence parent 200, commentaire 404, parent 404 retourne `inconclusive/resource_not_found`, `retryable=false` et aucun état canonique.
- `python3 -m py_compile scripts/run_forgejo_live_webhook_reconcile_emitted.py tests/test_forgejo_emitted_webhook_e2e.py scripts/run_webhook_reconcile_cli_e2e.py` — réussi.
- `rustfmt --edition 2024 --check src/webhook/reconcile.rs tests/webhook_reconcile_contract.rs tests/webhook_receiver_process_e2e.rs` — réussi.
- `git diff --check` — réussi.

Les sorties se trouvent dans les fichiers `*-contract.log`, `process-*.log` et les journaux de formatage/contrôle adjacents. Les tests processus utilisent le runner Bubblewrap avec espaces réseau et PID isolés. Ils vérifient aussi qu’aucun en-tête `Authorization` n’est envoyé et que les octets du journal webhook ne changent pas.

L’échec test-first du lot d’implémentation précédent a été observé dans le terminal mais n’a pas été conservé sous forme de journal brut. Les tests de régression conservés vérifient directement qu’un succès parent en cache ne masque pas un 404 lors de la relecture. Un premier essai du harnais de ce lot attendait six requêtes dans le test d’issue, alors que sa fixture HTTP est recréée pour chaque permutation; le compte du test a été corrigé à trois. Cet échec concernait le harnais, pas le comportement applicatif.

## Portée et limites

La fixture processus séquence les réponses; elle ne modifie pas réellement les ACL Forgejo pendant les requêtes. CONTINUATION-166 avait observé séparément qu’une révocation réelle sur Forgejo local rendait la route parent 404, mais ne synchronisait pas cette révocation au milieu de ces trois GET. La dernière relecture reste séquentielle et non atomique : les droits peuvent changer juste après. Ces tests portent sur ce chemin client/cache, pas sur toutes les routes Forgejo, tous les modèles de permissions ou l’isolation de production. Ils n’établissent aucun gain de productivité ou de coût.

Aucun code Forgejo n’a été copié ou modifié. Aucun modèle payant, AgentOS, service externe, publication ou système de production n’a été utilisé. Les sources, outils et versions sont épinglés dans [environment.txt](environment.txt) et [source-sha256.txt](source-sha256.txt); le [checkpoint cumulatif](../../docs/checkpoint.md) et la [matrice des refus](../../docs/refusal-evidence-matrix.md) décrivent les écarts ouverts.
