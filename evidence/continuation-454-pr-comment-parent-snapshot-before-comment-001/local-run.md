# CONTINUATION-454 — refuser un snapshot PR incomplet avant le GET du commentaire

Date : 29 octobre 2026. Worktree produit local : `/home/pc6/Documents/Codex/2026-10-21-mission-construire-une-forge-con-ue/.runtime/webhook-005-workspace`; base Git `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`, checkout fortement modifié. Aucun autre changement local n’a été écrasé.

## Contrat vérifié

Pour un événement `issue_comment` d’une conversation PR, le client relit l’API de la PR parente avant l’API enfant du commentaire. CONTINUATION-453 liait déjà `base.repo` avant le commentaire, mais le reste des champs PR n’était validé par `current_pull_request_fields` qu’après cette lecture enfant. Une tête invalide, un `draft` absent, un SHA base mal formé ou des reviewers mal formés pouvaient donc déclencher un GET commentaire avant le refus du snapshot parent.

Le contrat rouge `pull_request_comment_parent_rejects_invalid_head_before_reading_comment` construit une PR avec `head.sha="not-a-valid-commit-id"`. Avant le changement, le verdict final était déjà `InvalidCurrentStateField("head.sha")`, mais le compteur d’appels API était 2, alors qu’un parent PR invalide doit être refusé sans lire le commentaire. La sortie rouge est conservée dans `contract-red.log`.

Le runtime retourne maintenant un struct privé `CurrentPullRequestFields` depuis le parseur déjà utilisé pour les PR. `CurrentCommentParentSnapshot` retient ce résultat; `reconcile_issue_comment` le construit avec le snapshot initial du parent, avant `source.read_current(&resource)`, puis le réutilise pour la projection. Lorsque la suppression d’un commentaire répond 404, le parent est relu frais et ses champs PR sont recalculés avant de projeter `absent`. Le chemin PR ordinaire continue de réutiliser le même parseur, sans copier une seconde politique de validation.

## Résultats

- Contrat unitaire ciblé : **1/1** après implémentation; la cible `webhook_reconcile_contract` passe **111/111**.
- Processus receiver → inbox durable → binaire réel `forgejo-reconcile-local`, sous Bubblewrap avec namespaces réseau et PID privés : **1/1**. Une réponse PR loopback avec SHA head mal formé cause un seul `GET /pulls/24`, aucun GET `/issues/comments/916`, aucun en-tête Authorization, un JSON `inconclusive/invalid_current_state` avec le champ `head.sha`, aucune projection canonique, aucun écho de `untrusted webhook comment`, et le journal reste byte-for-byte identique.
- Régression précédente liée à CONTINUATION-453, base de dépôt étrangère : **1/1** après le refactor.
- Parcours positif existant PR-conversation → GET PR → GET commentaire : **1/1** après correction de sa fixture, qui omettait `draft` bien que le parseur courant l’exige.
- Suite `cargo test --locked --offline --all-targets -- --test-threads=1` : **39 cibles, 791 réussis, 0 échec, 214 ignorés**. Les tests processus dédiés restent ignorés dans cette suite et sont exécutés séparément.

Le reçu rouge et les trois journaux processus sont `contract-red.log`, `process-e2e.log`, `prior-base-refusal-e2e.log` et `pr-comment-positive-e2e.log`; le résumé all-targets est dans `test-summary.json`. Les symptômes, la cause et la correction de fixture positive figurent dans `positive-fixture-red-observed.txt`. Le résumé initial n’est pas présenté comme stdout brut du test.

## Compatibilité Forgejo et vérifications de projet

L’inventaire local épingle Forgejo officiel v17.0.0 à `0d791421f85ca13a63b665053d81ea765fbbc571`. Les fichiers de route, DTO, convertisseur et modèle concernés sont conservés propres dans le checkout scratch; leurs Git blob IDs et la confirmation de statut sont dans `source-reference.txt` et `docs/source-inventory.md` (section « Commentaires de PR — lecture canonique Forgejo v17.0.0 »). Aucun code Forgejo n’est copié.

Vérifications finales exécutées : `cargo fmt --all -- --check`, `git diff --check`, `PYTHONPYCACHEPREFIX=/tmp/cont454-pycache python3 -m py_compile scripts/run_webhook_reconcile_cli_e2e.py`, `python3 scripts/verify_cargo_license_state.py` et `python3 evidence/documentation-link-audit-001/verify_doc_links.py`. Rustfmt a d’abord signalé une seule ligne du nouveau test processus; `cargo fmt --all` l’a corrigée, puis le contrôle complet est passé. L’audit porte sur cinq documents canoniques, 1 678 liens locaux contrôlés et zéro cible manquante. Le manifeste de licence confirme `license=null`, `license_file=null`, `publish=[]`, décision `no_license_selected`, droits `not_assessed`, SPDX `NOASSERTION`; aucun champ de licence produit n’a été choisi. Les commandes exactes, versions, empreintes et résultats des manifestes sont dans les fichiers voisins du reçu.

Le script append-only `append_manifests.py` vérifie les index worktree et projet : respectivement **14 107** et **9 276 chemins uniques**, aucun chemin manquant, divergent, non haché courant ou dangereux. Il signale une entrée historique `legacy_unhashed_records` dans l’index worktree et aucune dans l’index projet. Les manifestes complets de chaque index et du paquet sont conservés; les empreintes des huit sources/documents touchés sont dans `source-sha256.txt`.

## Portée et limites

Les réponses Forgejo sont des fixtures HTTP loopback. Bubblewrap vérifie ce chemin de processus local, mais ne prouve ni une instance Forgejo live, ni l’isolation de tous les agents, ni un contrôle multi-hôte. Aucun LLM, AgentOS, GitLab, courriel, service externe, coût, publication ou changement de production n’a été utilisé dans ce lot. Aucun gain comparatif n’est déduit. Le scénario ajouté protège l’ordre validation-parent → GET-commentaire; il ne démontre pas le projet complet.
