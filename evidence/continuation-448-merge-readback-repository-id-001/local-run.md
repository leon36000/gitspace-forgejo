# CONTINUATION-448 — lier l’ID de dépôt entre les readbacks PR et issue

Date : 28 octobre 2026. Worktree : `.runtime/webhook-005-workspace`, branche `local/WEBHOOK-005-canonical-links`, HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. Le dépôt était déjà très modifié et contient des fichiers non suivis; rien n’a été nettoyé, réinitialisé, commité ou publié.

## Invariant et implémentation

CONTINUATION-447 liait les trois identifiants textuels de l’issue à la cible, mais ne comparait pas son ID numérique avec celui du dépôt base de la PR fusionnée. Forgejo v17.0.0 expose cet ID comme `int64` dans `Repository` et `RepositoryMeta`. Le gate exige maintenant que `base.repo.id` et `issue.repository.id` soient tous deux des entiers JSON positifs de type signé 64 bits et égaux. Il conserve aussi les comparaisons `owner`, `name`, `full_name`; un champ absent, non numérique, non positif ou différent rend le readback inconclusif.

Cette égalité est une cohérence entre deux snapshots actuels, pas une ancre numérique préconfigurée dans le profil cible. Le profil lie toujours l’owner/dépôt par texte. L’égalité ne prouve ni que la réponse est authentique ni que les deux vues API sont atomiques.

Le contrat `--capabilities` a changé; il est incrémenté à schema 29, `forgejo-reconcile-capabilities-v29`. L’opération `local_test_merge_authority` annonce les quatre champs d’issue obligatoires et la relation positive ID égale au `postmerge_pull_request_identity.base.repo.id`.

## Preuves

Le test `merge_readback_refuses_divergent_commit_and_issue_state` a d’abord échoué sur deux IDs positifs différents. Après implémentation, il passe 1/1 et refuse explicitement ID différent, nul, manquant/non-entier et ID base nul/manquant. Le test processus `merged_readback_with_mismatched_issue_repository_id_is_inconclusive` lance le vrai binaire sous Bubblewrap avec réseau/PID isolés. La fixture applique le POST, puis renvoie base ID 17 et issue ID 99 avec les noms textuels attendus. Le CLI garde `inconclusive / merge_outcome_not_confirmed`, un intent commencé et aucun intent résolu. E2E `n=1`; la fixture n’est pas Forgejo live.

Le run final `cargo test --locked --offline --all-targets -- --test-threads=1` passe 39 groupes : 786 réussis, 0 échec, 209 ignorés. L’E2E ciblé est explicitement exécuté séparément; les tests ignorés ne sont pas comptés comme runs. L’exécutable réel `--capabilities` est capturé et validé dans `capabilities.json`. Résultats et logs : `test-results.txt`.

## Cargo et limites

`cargo metadata --locked --offline --format-version 1 --no-deps` confirme `license=null`, `license_file=null`, `publish=[]`; le package garde `publish=false`. Le vérificateur garde `no_license_selected`, droits `not_assessed`, SPDX `NOASSERTION`. Aucune licence n’est choisie. Cela résout la représentation technique du choix non fait; les droits juridiques ne sont pas évalués.

Le manifeste local `SHA256SUMS` couvre 19 artefacts de preuve; les index append-only du worktree et du projet passent après ajout avec préfixes conservés et zéro chemin manquant ou divergent. Le run de tests vérifie la logique locale contre des fixtures loopback, pas Forgejo en exécution. Le POST de la fixture a déjà été appliqué; l’invariant empêche seulement le client d’annoncer une confirmation sur des IDs discordants, pas de compenser une fusion. Une première passe de contrôle final a échoué uniquement parce que mon parseur de journal de vérification utilisait une expression régulière mal échappée; le reçu `final-checks-aggregate-regex-error.log` la conserve et la passe corrigée est `final-checks.log`. Ce n’était pas un échec de compilation ou de test produit. Le parcours LLM/AgentOS, la comparaison conventionnelle, la fusion atomique contre tous les writers et la revue des droits/licences restent ouverts. Aucun modèle, service externe, coût, production, push ou credential n’a été utilisé.
