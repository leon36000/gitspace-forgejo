# CONTINUATION-203 — readback live d’une équipe reviewer sur PR d’organisation

Run final local du 27 octobre 2026, worktree `webhook-005-workspace`, base HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`, dépôt fortement modifié. Aucun commit, déploiement ou changement de production.

## Lacune ciblée

Le code de projection et le CLI avaient déjà des cas synthétiques avec équipes non vides (CONT-170/171), tandis que l’instance réelle de CONT-178 ne rendait que `requested_reviewers_teams=[]` sur un dépôt utilisateur. Dans le snapshot Forgejo officiel v17.0.0 commit `0d791421f85ca13a63b665053d81ea765fbbc571`, `routers/api/v1/repo/pull_review.go::apiReviewRequest` ne traite les noms d’équipes que si le dépôt appartient à une organisation (`pull_review.go:908-930`). La route d’organisation et de création d’équipe est enregistrée dans `routers/api/v1/api.go:1280-1307`. `services/issue/assignee.go::IsValidTeamReviewRequest` exige qu’une équipe puisse lire un dépôt privé (`assignee.go:160-193`). `services/convert/convert.go::ToTeams` construit les champs `permission` et `units_map` à partir des permissions d’unité (`convert.go:365-399`). Les fichiers consultés ne diffèrent pas du commit de base dans le scratch; aucun code Forgejo n’a été copié.

## Parcours et résultat

Le nouveau harnais [`run_forgejo_live_org_team_reviewer_readback.py`](../../scripts/run_forgejo_live_org_team_reviewer_readback.py) démarre Forgejo uniquement sur `127.0.0.1:3130` avec un état neuf sous `/tmp`, provisionne des comptes de test, puis crée une organisation privée, une équipe avec `repo.pulls=write`, un membre reviewer, un dépôt privé et un commit candidate via l’API locale. Il crée ensuite une PR ouverte, demande `team_reviewers=["reviewers"]` et relit la même PR. La fonction `validate_team_readback` lie le HTTP 201 à la projection `{id,name}` du normaliseur Python déjà présent dans `run_forgejo_live_webhook_reconcile_emitted.py`.

Commande finale :

```text
python3 scripts/run_forgejo_live_org_team_reviewer_readback.py --forgejo-binary /tmp/forgejo-17.0.0 --state-dir /tmp/forgejo-team-review-cont203-state-003 --evidence-dir evidence/continuation-203-live-org-team-reviewer-readback-003
```

Verdict `passed`; Forgejo a accepté la demande en HTTP 201 et son GET courant a rendu exactement `[{"id":2,"name":"reviewers"}]` pour la PR #1 ouverte, non fusionnée, de `forge-team-org/team-review-readback`. Le binaire annonce Forgejo `17.0.0+gitea-1.22.0`, Go 1.26.9, SHA-256 `e5ec437cd6357e8d1b4580cb47ab1daa58a32b33971a63db63446a535d33adb7`; le commit exact du binaire n’est pas établi. Réponse structurée : [`result.json`](result.json); arrêt du serveur et listener absent : [`cleanup.json`](cleanup.json). Le répertoire d’état final reste sous `/tmp`, mode 0700. Aucun modèle ni agent n’a été appelé.

## Essais, correction du harnais et vérifications

- Le premier dossier `continuation-203-live-org-team-reviewer-readback-001` contient un run exploratoire réussi avant les assertions PR plus strictes.
- Le run `...-002` s’est arrêté au contrôle groupé de la réponse Team. Le JSON Team n’a pas été sauvegardé, donc le champ fautif (ID, nom, `includes_all_repositories`, permission ou unités) n’est pas isolé. La permission agrégée `write` était une hypothèse injustifiée pour des unités mixtes; le code source Forgejo calcule ce champ global depuis le minimum de ses unités. Le harnais vérifie maintenant l’unité utile `repo.pulls=write`, présente dans le readback Team final. Les dossiers d’essais sont préservés et hashés par [`prior-attempts.sha256`](prior-attempts.sha256).
- Le run final vérifie aussi que la PR relue est le numéro demandé, ouverte, non fusionnée et rattachée au dépôt de base attendu.
- Les tests ciblés du validateur : 4/4 réussis; `py_compile` et `git diff --check` passent. Sorties conservées dans `focused-tests.log`, `pycompile.log` et `diff-check.log`.

## Limites

Cette preuve live traverse l’API de Forgejo et le normaliseur Python, pas le receiver, le journal webhook ou le CLI Rust. Aucun webhook n’est configuré ni observé; le profil du CLI Rust reste lié à un dépôt fixe, ce qui empêche ce scénario d’être consommé tel quel par l’adaptateur. La lecture live non vide par ce CLI reste à démontrer. Une organisation, une équipe et une instance (`n=1` final); pas d’évaluation d’accès multi-tenant, de merge, de vérificateur indépendant, d’AgentOS, de LLM, de tokens/coût ou de comparaison de productivité. Aucun service externe, secret privé, coût nouveau ou code upstream n’est utilisé.

## Reproduction et empreintes

Commandes, versions et empreintes de code source : [`commands.txt`](commands.txt), [`source-hashes.txt`](source-hashes.txt). Le snapshot Forgejo local est au commit officiel indiqué ci-dessus, mais son scratch de travail est dirty pour d’autres POC; l’intégrité des fichiers pertinents est associée à leurs hashes. La matrice de couverture est mise à jour dans [`docs/refusal-evidence-matrix.md`](../../docs/refusal-evidence-matrix.md); le checkpoint reste explicite sur les critères non démontrés.
