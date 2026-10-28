# CONTINUATION-317 — révocation d’avis après le contrôle de merge

Date : 27 octobre 2026.

## Question

Le chemin API normal de merge Forgejo refuse-t-il de fusionner si l’unique approbation comptée au preflight est dismissée pendant que le même POST merge est en cours?

## Source et environnement

- Source officielle Forgejo v17.0.0 au commit `0d791421f85ca13a63b665053d81ea765fbbc571`.
- Worktree détachée `/tmp/forgejo-v17-merge-review-race-001` à ce commit, instrumentée par le seam test-only décrit dans [CONTINUATION-316](../continuation-316-forgejo-merge-review-policy-race-001/local-run.md). La worktree dirty `/tmp/forgejo-v17-base-cas-poc` n’a pas servi.
- Le test `tests/integration/api_pull_merge_race_test.go::TestAPIMergePullRequestCanUseApprovalBeforeConcurrentDismissal` crée une PR, une protection d’une approbation, puis suspend le POST API après le succès de `CheckPullMergeable`.
- SQLite via `tests/sqlite.ini`, Go `1.27.3 linux/arm64`, Git `2.43.0`, `CGO_ENABLED=1`. `git-lfs` est absent.

## Protocole et observation

Une revue officielle `APPROVED`, liée exactement au SHA de tête, est créée par `user2`, propriétaire du dépôt et compte de la whitelist. `user1` soumet le POST normal `Do=merge` avec le SHA exact, sans `force_merge`. Le hook retient la requête après `CheckPullMergeable`. Le propriétaire utilise ensuite `POST /repos/{owner}/{repo}/pulls/{index}/reviews/{id}/dismissals`; l’API retourne HTTP 200 et le readback de la revue dit `dismissed=true`. Le POST merge retenu est libéré : il retourne HTTP 200; le GET de la PR indique `HasMerged=true`; le GET de la même revue confirme `dismissed=true`.

Le test ciblé passe cinq exécutions consécutives (`-count=5`) et passe également avec le détecteur de courses Go (`-race -count=1`). Cela établit que, dans cette route et cette copie instrumentée, le merge poursuit après la révocation de l’unique approbation précontrôlée. Avec CONT-316, le lot établit séparément le même résultat après hausse concurrente du seuil.

## Commandes et sources

Commandes, versions, hashes amont/scratch et sorties complètes figurent dans les fichiers voisins. Les symboles lus sont `routers/api/v1/api.go` (route dismissal), `routers/api/v1/repo/pull_review.go::DismissPullReview/dismissReview`, `services/pull/review.go::DismissReview` et `models/issues/review.go::DismissReview`, en plus de `MergePullRequest/CheckPullMergeable` du reçu CONT-316. Le Make wrapper SQLite n’a pas été utilisé à cause du précontrôle `git-lfs` absent; l’appel Go ciblé a été exécuté directement.

Les manifestes locaux, worktree et projet sont append-only. `verify_manifests.py` valide leurs préfixes et lignes les plus récentes; `SHA256SUMS.current` fournit les empreintes uniques des fichiers de ce reçu pour `sha256sum -c`.

## Limites

Le serveur utilisé est le harnais d’intégration Forgejo avec SQLite et push Git réel de fixture, mais la route de merge est instrumentée par un hook test-only ajouté après le retour du contrôle. Ce n’est pas une preuve sur un build non instrumenté, une instance de production, PostgreSQL, cluster ou autres writers. Seule la route API normale est testée; Web, auto-merge, manuel, force-merge et révocation d’avis dans des fenêtres différentes restent non examinés. Ce test ne démontre pas si un redémarrage/crash ou un déploiement réel permettrait la même interleaving.

Aucun code Forgejo, patch, secret, service externe ou LLM n’a été utilisé dans le produit; l’exécution a été locale et sans coût fournisseur. Cette preuve ne tranche pas à elle seule le fork : la suite doit comparer les routes et writers de ref, puis le coût upstream d’un contrôle atomique commun. Le gate ne peut pas appeler cette pré-vérification seule une autorisation durable.
