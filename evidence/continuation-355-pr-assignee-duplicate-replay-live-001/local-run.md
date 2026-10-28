# CONTINUATION-355 — dédupliquer un rejeu live `pull_request/assigned` après reprise

Date : 28 octobre 2026. Worktree `local/WEBHOOK-005-canonical-links`, HEAD de base `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`; arbre préexistant fortement modifié et préservé.

## Lacune ciblée

Le runner `scripts/run_forgejo_live_webhook_reconcile_emitted.py` rejouait toujours la première livraison, qui était l’ouverture d’une issue. Le scénario précédent avait ajouté des événements réels `pull_request/assigned`, mais ne les utilisait pas pour vérifier la déduplication après crash/reprise. Le nouveau sélecteur choisit le premier événement d’assignation PR quand ce scénario est activé; les autres parcours conservent l’ouverture d’issue comme cible de repli. Le reçu indique maintenant header d’événement et action rejoués.

Test-first : le test de sélection a d’abord échoué car le sélecteur n’existait pas; après implémentation, les quatre tests `DuplicateDeliveryReplayTests` passent. La suite Python passe 155/155; syntaxe AST, présence de l’option CLI, `git diff --check` et contrôle des espaces finaux sur les deux fichiers Python passent. Sorties : [`tests.log`](tests.log), [`checks.log`](checks.log).

## Résultat live

Forgejo local jetable annonce `17.0.0+gitea-1.22.0`, binaire SHA-256 `e5ec437cd6357e8d1b4580cb47ab1daa58a32b33971a63db63446a535d33adb7`. Le run émet l’assignation d’une PR, arrête le receiver par SIGKILL, le redémarre avec le même journal puis renvoie la charge enregistrée de `pull_request/assigned` sous son ID d’origine. Le receiver répond HTTP 202, `duplicate=true`, `reconcile_required=true`, séquence 7. Le journal reste à 9 enregistrements; taille et SHA-256 avant/après sont égales. Le run passe, le hook est désactivé, Forgejo et le receiver terminent et leurs ports d’écoute sont absents. Reçu : [`result.json`](forgejo-run-001/result.json); provenance : [`provenance.json`](forgejo-run-001/provenance.json).

La charge du rejeu est JSON re-sérialisée depuis l’objet du journal signé par le receiver; le harnais ne conserve pas ni ne compare les octets bruts de la requête initiale. Le sender de rejeu utilise uniquement le secret synthétique de cette instance jetable.

## Commande et portée

```text
PYTHONDONTWRITEBYTECODE=1 python3 scripts/run_forgejo_live_webhook_reconcile_emitted.py --forgejo-binary /tmp/forgejo-17.0.0 --repo-root "$PWD" --state-dir /tmp/forgejo-cont-355-pr-assignee-state-001 --evidence-dir "$PWD/evidence/continuation-355-pr-assignee-duplicate-replay-live-001/forgejo-run-001" --change-pull-request-assignees
```

Ce run démontre le chemin du receiver local pour un doublon d’assignation PR issu de Forgejo et la non-extension du journal après reprise. Il ne démontre pas un retry émis à nouveau par Forgejo, une panne de disque, une concurrence multi-processus/multi-hôte, une barrière d’egress, un vrai credential de fusion, un LLM/AgentOS, GitLab, merge ou gain comparatif. Aucun accès externe, production, coût, commit ou publication n’a eu lieu.
