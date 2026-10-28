# CONTINUATION-203-004 — relire explicitement la visibilité de l’organisation

Run final du 27 octobre 2026 dans le worktree `webhook-005-workspace`, base HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`, fortement modifié. Ce reçu complète le run API non vide déjà documenté en [CONTINUATION-203-003](../continuation-203-live-org-team-reviewer-readback-003/local-run.md).

## Contrôle ajouté

En revue du reçu 003, j’ai constaté que sa valeur `organization.visibility=private` provenait de la demande envoyée, sans assertion sur la réponse Forgejo. Le harnais [`run_forgejo_live_org_team_reviewer_readback.py`](../../scripts/run_forgejo_live_org_team_reviewer_readback.py) vérifie maintenant que la réponse HTTP 201 de création de l’organisation contient bien `visibility=private`, et sérialise la valeur observée. Il lie également le readback de PR au numéro, à l’état ouvert, au statut non fusionné et au dépôt de base attendu.

## Run local

Commande exacte :

```text
python3 scripts/run_forgejo_live_org_team_reviewer_readback.py --forgejo-binary /tmp/forgejo-17.0.0 --state-dir /tmp/forgejo-team-review-cont203-state-004 --evidence-dir evidence/continuation-203-live-org-team-reviewer-readback-004
```

Forgejo local v17.0.0 retourne `visibility=private` pour `forge-team-org`; l’équipe `reviewers` est créée avec `repo.pulls=write`, rattachée à une membre, et affectée à la PR #1 du dépôt privé `forge-team-org/team-review-readback`. La demande `team_reviewers` répond HTTP 201. Le GET courant de cette PR ouverte et non fusionnée retourne exactement `[{"id":2,"name":"reviewers"}]`; le normaliseur Python existant valide ce readback.

Le binaire annonce `17.0.0+gitea-1.22.0`, Go 1.26.9, SHA-256 `e5ec437cd6357e8d1b4580cb47ab1daa58a32b33971a63db63446a535d33adb7`; commit exact du binaire inconnu. Le snapshot source Forgejo officiel v17.0.0 est au commit `0d791421f85ca13a63b665053d81ea765fbbc571`; les fichiers pertinents ne diffèrent pas de ce commit dans le scratch. Les hashes source figurent dans [`source-hashes.txt`](source-hashes.txt).

Les quatre tests unitaires du validateur passent, `py_compile` et `git diff --check` réussissent. Le test ne configure aucun webhook et n’invoque ni receiver ni CLI Rust; il s’arrête à l’API live et à la projection Python. Aucun agent/LLM, coût ou service externe. Résultat et arrêt confirmé sans listener restant : [`result.json`](result.json), [`cleanup.json`](cleanup.json). L’état local privé du test reste sous `/tmp`, mode 0700.

L’instance n=1 confirme un readback REST réel d’équipe non vide; elle ne prouve pas encore la consommation par le CLI Rust, une delivery webhook, l’autorité de fusion, la valeur d’un agent ou un gain comparatif. La prochaine étape ciblée est de vérifier si un profil Rust fixe d’organisation peut être ajouté sans rendre owner/repository arbitraires; voir [`docs/checkpoint.md`](../../docs/checkpoint.md).
