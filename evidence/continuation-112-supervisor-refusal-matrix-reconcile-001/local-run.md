# CONTINUATION-112 — rapprocher la matrice des reçus superviseur actuels

Date : 26 octobre 2026. Worktree : `/home/pc6/Documents/Codex/2026-10-21-mission-construire-une-forge-con-ue/.runtime/webhook-005-workspace`. HEAD lu : `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`; checkout déjà fortement modifié. Aucun reset, nettoyage, suppression, commit, serveur Forgejo, LLM, appel externe ou service de production n’a été lancé.

## Lacune documentaire

La matrice indiquait encore qu’aucune capacité superviseur positive ni voie de fusion live n’existait. Cette conclusion était dépassée par CONTINUATION-110 et le parcours local MISSION-012 supervised 009. Le reçu 009 rapporte 14/14 contrôles superviseur, une PR Forgejo fusionnée, une validation indépendante 2/2 sur le commit de merge réellement relu, et les sondes du processus agent refusées sur les routes API, Web et `git-receive-pack` pendant que le gate distinct lit et fusionne. Le même rapport précise que le processus agent n’a pas le credential, que son adaptateur est déterministe et que l’exclusion des writers dépend toujours de la politique du superviseur (`supervisor_policy_independently_verified=false`).

Je n’ai pas rejoué Forgejo ni les tests du parcours. J’ai corrigé uniquement les lignes de la matrice sur le reçu mono-writer et les capacités d’egress, en gardant visibles les limites : `n=1`, pas de LLM/AgentOS ni de benchmark, trois routes sondées, binaire épinglé sans commit de build amont, et absence de preuve indépendante qu’aucun writer ne peut rejoindre le namespace protégé.

## Manifestes append-only

La vérification fournie avec le parcours 009 a d’abord échoué en lecture seule avec `project index does not hash the current worktree index`. Le manifeste enfant avait reçu des entrées ultérieures après le dernier hash parent. Le présent lot enregistre les préfixes exacts avant modification dans `manifest-prefix-before.json`, ajoute les hashes courants au manifeste enfant et au manifeste parent sans réécrire leur historique, puis vérifie les deux préfixes et tous les hashes indexés. Le script de mise à jour inclut également la matrice modifiée et le checkpoint.

Deux erreurs de préparation du nouveau reçu sont également documentées : le générateur du préfixe a d’abord écrit les deux caractères littéraux `\\n`, ce qui a arrêté le lecteur JSON avant toute écriture de manifeste; après correction, l’appender a correctement refusé l’absence initiale du fichier local `SHA256SUMS`. Le préfixe JSON a été réparé puis le manifeste local vide créé avant le premier append réussi. Aucune entrée globale n’a été changée par ces essais refusés.

## Vérifications exécutées

- `python3 -B evidence/mission-012-live-agent-webhook-supervised-009/verify_append_only_indexes.py` : avant réparation, sortie 1 et diagnostic ci-dessus. Aucun fichier n’a été modifié par cette vérification.
- `python3 -B evidence/continuation-112-supervisor-refusal-matrix-reconcile-001/append_manifests.py` : 6 fichiers de preuve dans le manifeste local, 9 nouvelles entrées dans le manifeste worktree et 10 dans le manifeste parent lors du premier append complet; préfixes préservés.
- `python3 -B evidence/continuation-112-supervisor-refusal-matrix-reconcile-001/verify_append_only_indexes.py` : contrôle indépendant des préfixes, de la couverture locale et des index worktree/projet.
- La vérification archivée par CONTINUATION-009 sur ses propres fichiers passe de nouveau après le rapprochement des index.
- `git diff --check` : sans sortie.

Aucun test produit n’a été relancé : le changement est documentaire et aucune source exécutable n’a été modifiée. La mission reste incomplète. La comparaison contrôlée exige encore un runtime d’agent et une limite de coût numérique fiable explicitement autorisée; la politique superviseur ne prouve pas l’exclusion universelle des writers.
