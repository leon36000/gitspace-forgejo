# FORGEJO-WEBHOOK-BRANCH-ZERO-CURRENT-SHA-001 — refuser un snapshot API de branche à OID nul

Date : 26 octobre 2026. Travail local dans le worktree `webhook-005-workspace`, basé sur `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`; dépôt fortement modifié avant ce lot. Aucun code Forgejo n’a été copié, aucun service externe ou modèle utilisé, et aucune instance de production touchée.

## Défaut reproduit et correction

La réconciliation d’une branche vérifiait que `commit.id` avait 40 ou 64 chiffres hexadécimaux, puis pouvait retourner `state=present` avec un SHA entièrement nul. Ce sentinel indique l’absence d’objet Git; il ne peut pas nommer le commit courant d’une branche que l’API vient de lire. Les champs `push.before` et `push.after` ont un contrat différent : l’OID nul y signale la création ou la suppression d’une ref et doit rester accepté.

Le test `branch_snapshot_requires_the_requested_name_and_a_valid_commit` a été étendu avec les réponses d’API à 40 et 64 zéros. Avant le changement, le test a échoué précisément parce que la réconciliation retournait `Ok(ReconciledState { state: "present", head_sha: Some("000…") })`. La validation du `commit.id` courant appelle maintenant `valid_nonzero_commit_sha`; la validation générale du corps push n’a pas changé.

## Vérification par les processus produits

Le scénario processus `actual_reconcile_cli_process_refuses_zero_current_branch_commit` a utilisé le vrai `forgejo-webhook-local` pour accepter un push signé de fixture où `before` vaut zéro et `after` désigne un commit valide. Le vrai `forgejo-reconcile-local`, lancé dans le même bac à sable Bubblewrap avec réseau et PID isolés, a lu une réponse HTTP 200 d’une fixture loopback contenant un `commit.id` nul. Il termine avec le verdict `inconclusive`, `invalid_current_state`, champ `commit.id`, `retryable=false`, sans `canonical_state`; le journal reste octet pour octet inchangé. Un seul GET sur la branche configurée est observé, sans en-tête Authorization.

Résultats : test contrat ciblé rouge avant correction (0 réussi, 1 échoué attendu, code 101); cible `webhook_reconcile_contract` après correction 44 réussis, 0 échec; E2E processus isolé 1 réussi, 0 échec, 93 filtrés. Rustfmt, diff-check, compilation Python du runner et garde Cargo passent. Versions, commandes, logs et empreintes des sources figurent dans ce dossier.

## Limites

L’API est un serveur HTTP synthétique; Forgejo v17 n’a pas émis le webhook pendant ce run. Le premier processus est le vrai receiver; l’adaptateur API est exercé contre une réponse fixture et aucun credential n’est fourni. Ce scénario déterministe n=1 prouve la règle de parsing et la sortie structurée sur un OID nul, pas l’exhaustivité des réponses API ou des systèmes Git.

Recontrôle d’intégration local : `AgentRuntimeV1` n’a encore que `FixtureRuntime` comme implémentation observée; `ollama`, `llama-server`, `llama-cli`, `lmstudio`, `lms`, `opencode`, `aider` et `goose` ne sont pas dans le PATH. Aucun agent LLM/AgentOS n’a été lancé. Le garde Cargo confirme toujours `license=null`, `license_file=null`, `publish=[]`, `no_license_selected`, droits `not_assessed`, marqueur personnalisé `NOASSERTION`; aucune licence produit n’est attribuée.

La prochaine intégration locale utile est le trajet réel Forgejo v17 → commentaire sur une issue ordinaire → receiver → journal → réconciliation vers `/issues/{n}`. Les preuves existantes synthétisent la forme du commentaire et son routage; elles disent explicitement qu’aucun hook live de ce cas n’a été observé. Ce parcours doit utiliser une instance de test isolée et ne pas réutiliser une instance de production.
