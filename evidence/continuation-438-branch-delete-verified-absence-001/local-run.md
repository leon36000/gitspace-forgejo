# CONTINUATION-438 — confirmer l’absence d’une branche native

Date : 2026-10-28. Worktree : `.runtime/webhook-005-workspace`, révision de base `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`; aucun commit ou appel de service externe n’a été fait.

## Changement

Pour un webhook natif `delete` de branche, une première réponse API 404 ne suffit plus à déclarer la ressource absente. Le réconciliateur vérifie d’abord l’accès au dépôt configuré avec un GET exact `/repos/{owner}/{repo}`, puis compare `full_name`, `name` et `owner.login` aux paramètres de confiance. Il relit ensuite la branche sans réutiliser une réponse mise en cache. Un second 404 donne un snapshot `absent`; une branche recréée est projetée comme `present`; une identité invalide ou un accès non confirmé reste inconclusif. Les lectures d’autres types d’événement ne changent pas. L’allowlist ajoute seulement ce GET, sans body, au dépôt déjà configuré.

Dans un batch `create` puis `delete` où le ref est absent, le résultat du `create` reste inconclusif, celui du `delete` décrit l’absence vérifiée et l’enveloppe du batch demeure inconclusive. Un 404 sur un `push` n’acquiert pas cette interprétation.

## Vérifications

`capture_verification.py` exécute hors ligne les commandes consignées dans `verification-results-attempt-004.json` et garde sortie et code de retour dans des fichiers distincts. Le premier lancement du collecteur avait un mauvais niveau de parent pour localiser la racine du worktree; `*-attempt-001.*` consigne l’échec de découverte (aucun test n’a été exécuté par cette tentative). Après ajout du scénario batch, l’essai 003 a détecté un écart rustfmt; le format a été corrigé puis toutes les commandes ont été rejouées. Les dix vérifications finales de l’essai 004 ont le code de retour 0 :

- `webhook_reconcile_contract` : 108 réussis, aucun échec; un nouveau test de batch confirme que create reste inconclusif quand delete lit une branche absente.
- Politique de route exacte pour le GET du dépôt : 1 réussi.
- Cible `webhook_receiver_process_e2e` hors sandbox : 44 réussis, 163 ignorés; les scénarios utilisant le port fixe 3130 restent marqués `#[ignore]` dans l’exécution générale.
- Trois scénarios de processus exécutés par `scripts/run_webhook_reconcile_cli_e2e.py` dans Bubblewrap avec namespaces réseau et PID privés : 3/3. Le succès simple observe branche 404 → dépôt 200 d’identité exacte → branche fraîche 404, trois tentatives et canonical state `absent`; le batch create/delete conserve le create inconclusif, rend le delete absent et l’enveloppe inconclusive après les trois mêmes GET. Le refus d’identité étrangère s’arrête après deux GET en `inconclusive/upstream_invalid_response`. Tous les scénarios n’envoient pas Authorization, ne divulguent pas les marqueurs privés de la fixture et gardent l’inbox byte-identique.
- `tests/test_forgejo_emitted_webhook_e2e.py` : 192 réussis.
- Compilation de vérification `cargo check --locked --offline --all-targets`, rustfmt, `git diff --check`, garde de métadonnées Cargo sans choix de licence et audit des liens canoniques : réussis; détails exacts dans les logs individuels.

## Limites

L’API est une fixture HTTP loopback synthétique et le webhook est signé localement par une clé de test. Aucun événement natif ou GET d’une instance Forgejo n’est exercé ici; le runner d’intégration Forgejo existant devra être validé sous cette nouvelle sémantique lors d’un lot séparé. Les GET successifs n’offrent ni transaction ni CAS : `absent` signifie seulement qu’une lecture fraîche a retourné 404 après vérification de l’identité du dépôt. L’absence peut changer immédiatement après. Les 162 tests de processus ignorés restent hors de la commande générale; les deux scénarios de ce lot ont toutefois été exécutés explicitement en sandbox.

Aucune intégration AgentOS/LLM, preuve de gain, analyse de coût ou décision de licence n’est créée par ce lot. Cargo reste `license=null`, `license_file=null`, `publish=[]`; droits de redistribution `not_assessed`, SPDX `NOASSERTION`.

Versions du système et commandes : `environment.txt`; résultats détaillés et sorties finales : `verification-results-attempt-004.json` et `*-attempt-004.log`. Les essais antérieurs sont conservés en append dans le paquet de preuve.
