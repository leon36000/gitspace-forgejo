# MERGE-015 — audit de la précondition de base de l’API de fusion Forgejo

**Date : 23 octobre 2026.** Question examinée : le POST de fusion Forgejo peut-il être lié atomiquement à la base qui a été vérifiée par le gate local ?

## Source épinglée

Le dépôt officiel `https://codeberg.org/forgejo/forgejo.git`, tag `v17.0.0`, a été interrogé avec `git ls-remote --refs`; le checkout détaché donne `HEAD = 0d791421f85ca13a63b665053d81ea765fbbc571`. Les références ci-dessous pointent vers ce commit. Ce SHA est celui de la source examinée; aucun lien de build entre ce checkout et le binaire local n’a été établi.

## Constat dans le code

- `routers/api/v1/repo/pull.go:905–906,1021` désérialise le formulaire puis passe `form.HeadCommitID` au service de merge.
- `services/forms/repo_form.go:457–469` définit `MergePullRequestForm`. Il expose `head_commit_id`, mais aucune précondition de SHA de base.
- `services/pull/merge_prepare.go:46–73` prépare le dépôt temporaire et compare, si demandé, le SHA `tracking` de la tête à `expectedHeadCommitID`. Cette comparaison porte sur la tête de PR, pas sur la base vérifiée auparavant par le gate.
- `services/pull/temp_repo.go:132–143` ajoute le dépôt de base comme remote puis récupère la branche de base courante dans le dépôt temporaire.
- `services/pull/merge.go:317–397` calcule le merge depuis ce dépôt temporaire puis pousse la branche préparée. Le push Git non fast-forward refuse un déplacement concurrent intervenu après le fetch interne de Forgejo.

## Conclusion et limite

L’API publique et le formulaire de ce tag ne reçoivent pas le SHA de base attendu par le vérificateur. Le push ordinaire protège le ref contre une perte de mise à jour après le fetch interne de Forgejo, mais ne prouve pas que la base utilisée pour calculer le merge est celle testée auparavant : si `main` avance après le dernier GET du gate et avant le fetch interne, puis que le contrôle de merge Forgejo accepte cet état (par exemple lors d’une incohérence Git/DB comme celle exercée dans MERGE-011), le merge peut être préparé sur la nouvelle base. Le re-read MERGE-014 réduit cette fenêtre sans rendre la décision atomique.

L’option autour de Forgejo reste conditionnée à une autorité de merge unique effectivement imposée côté Forgejo, avec tous les chemins d’écriture couverts. Le verrou du processus local ne protège que les writers coopératifs du même hôte. Si cette contrainte ne peut pas être imposée, il faut une précondition de base côté serveur; un client seul ne peut pas la simuler.

## Couverture et reproduction

Source lue : `routers/api/v1/repo/pull.go`, `services/forms/repo_form.go`, `services/pull/merge_prepare.go`, `services/pull/temp_repo.go`, `services/pull/merge.go`. Aucun build ni test Go n’a été lancé pour cet audit; la machine ne dispose pas de Go. Cette lecture ne démontre pas la version exacte du binaire de fixture.

Commandes : `git ls-remote --refs https://codeberg.org/forgejo/forgejo.git refs/tags/v17.0.0`; `git clone --depth 1 --filter=blob:none --branch v17.0.0 --sparse https://codeberg.org/forgejo/forgejo.git /tmp/forgejo-v17-merge-audit`; `git rev-parse HEAD`; l’expansion sparse `git sparse-checkout add modules/structs` puis `git sparse-checkout add services/forms`; `nl -ba` sur les fichiers et lignes cités. Le checkout sparse initial comprenait `routers/api/v1/repo`, `models/issues` et `services/pull`. SHA-256 des cinq fichiers examinés : `routers/api/v1/repo/pull.go` `10586bae76ce9976cfdd44bec2196bd74216e5555a11eae78ca6d41867023dcb`; `services/forms/repo_form.go` `30e7801ec62cd3cd4cba5448c4fad232a3cb63b8337cb4e3dd54f34fe4411fb6`; `services/pull/merge_prepare.go` `bce4ae33fbb4ad939a30332f46b653640f6486ab963e184097ac0c60d9100f1a`; `services/pull/temp_repo.go` `517d5b58427bb569512c27987b0691cfd7cb7dcd7cef14e74089e2f02a204c65`; `services/pull/merge.go` `43d724e4bcdcad0d48ca6b04430b18612c1008af86a374afa932b23d83659ac6`.

Sources primaires : [route API et appel au service](https://codeberg.org/forgejo/forgejo/src/commit/0d791421f85ca13a63b665053d81ea765fbbc571/routers/api/v1/repo/pull.go), [formulaire de merge](https://codeberg.org/forgejo/forgejo/src/commit/0d791421f85ca13a63b665053d81ea765fbbc571/services/forms/repo_form.go), [vérification de la tête](https://codeberg.org/forgejo/forgejo/src/commit/0d791421f85ca13a63b665053d81ea765fbbc571/services/pull/merge_prepare.go), [fetch de la base](https://codeberg.org/forgejo/forgejo/src/commit/0d791421f85ca13a63b665053d81ea765fbbc571/services/pull/temp_repo.go), [calcul et push](https://codeberg.org/forgejo/forgejo/src/commit/0d791421f85ca13a63b665053d81ea765fbbc571/services/pull/merge.go).
