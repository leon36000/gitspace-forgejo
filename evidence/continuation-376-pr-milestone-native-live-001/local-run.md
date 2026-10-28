# CONTINUATION-376 — milestones de PR émis nativement par Forgejo

Date : 28 octobre 2026. Worktree `/home/pc6/Documents/Codex/2026-10-21-mission-construire-une-forge-con-ue/.runtime/webhook-005-workspace`, branche `local/WEBHOOK-005-canonical-links`, base HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. Checkout fortement modifié; les fichiers existants ont été préservés. Aucun stage, commit, push, changement de remote ou accès GitLab.

## Lacune et changement

Les contrats Python/Rust et les scénarios processus connaissaient déjà les transitions `pull_request/milestoned` / `demilestoned`, et CONT-365 avait observé des milestones natifs pour une issue. Aucune preuve dans l’état de départ ne reliait ces deux transitions PR à une émission réelle Forgejo puis au réconciliateur.

Le harnais `scripts/run_forgejo_live_webhook_reconcile_emitted.py` expose maintenant `--change-pull-request-milestones`. Dans l’instance jetable, il crée un milestone, l’assigne via l’API d’issue associée à la PR, lit la PR via l’API Pull Request, capture l’événement natif, retire le milestone, vérifie le nouvel événement et la lecture API, puis inclut les deux livraisons dans la réconciliation batch et les preuves structurées. Les snapshots d’événements ne sont pas traités comme état canonique. Le test Python ajoute l’option CLI et refuse un clear dont le snapshot diffère ou dont la lecture API courante reste assignée.

Test-first : le nouveau test CLI échouait car l’option n’existait pas; les deux tests du validateur échouaient car il n’existait pas non plus. Après changement, les trois passent. La première invocation de unittest utilisait un nom de classe erroné et a échoué pendant la découverte; la commande correcte a ensuite produit le rouge attendu.

## Parcours observé

Commande exacte :

```text
python3 scripts/run_forgejo_live_webhook_reconcile_emitted.py --forgejo-binary /tmp/forgejo-17.0.0 --state-dir /tmp/forgejo-cont-376-pr-milestone-live-001 --evidence-dir <checkout>/evidence/continuation-376-pr-milestone-native-live-001/forgejo-run-001 --change-pull-request-milestones --batch-pull-request-events
```

Code de sortie 0, verdict `passed`. Le binaire local annonce Forgejo `17.0.0+gitea-1.22.0`, SHA-256 `e5ec437cd6357e8d1b4580cb47ab1daa58a32b33971a63db63446a535d33adb7`; l’environnement complet est dans `environment.txt`. La PR jetable no 2 a reçu `milestoned` à la séquence 7 puis `demilestoned` à la séquence 8; les deux tâches Forgejo ont été livrées au receiver avec HTTP 202 et des UUID distincts. Les lectures API après chaque transition ont retourné le milestone assigné puis `null`.

Le batch réel contient cinq livraisons PR; le résultat agrégé indique deux lectures upstream au total. Pour chacune des deux livraisons milestone, la sortie contient `current_state_read` et la projection actuelle `unassigned`, avec provenance `current_forgejo_resource_api` et fraîcheur `current_api_read`. Ainsi le premier événement `milestoned`, même s’il porte l’identité assignée, réconcilie vers l’état courant `unassigned` après le retrait. Les réponses détaillées, événements reçus, tâche UUID→delivery, réponse d’identifiant invalide et arrêt des processus sont sous `forgejo-run-001/`; le résumé ciblé est `forgejo-run-001/pull-request-milestone-change-reconciliation.json`.

Le harnais confirme que le hook a été désactivé, que Forgejo et le receiver sont arrêtés et que leurs listeners suivis sont absents. Le répertoire d’état privé de la fixture reste sous `/tmp`; il ne contient aucune donnée de projet ou credential externe. L’allowlist du hook est loopback seulement.

## Source et vérifications

Le clone officiel Forgejo dans `/tmp/forgejo-v17-base-cas-poc` est épinglé au commit `0d791421f85ca13a63b665053d81ea765fbbc571`. Revue source ciblée : `services/webhook/notifier.go::IssueChangeMilestone` branche sur `issue.IsPull` et construit un `PullRequestPayload` avec l’action native; `modules/structs/pull.go::PullRequest.Milestone` est le champ wire. Les hashes/objets Git sont dans `source-sha256.json`; ces deux fichiers upstream étaient propres à l’instantané de référence. Le binaire runtime confirme sa version et son hash, mais son commit de compilation précis n’est pas établi. Aucun code Forgejo n’a été copié.

Vérifications fraîches : `python3 -m unittest tests.test_forgejo_emitted_webhook_e2e -v` — 166/166; `py_compile` des deux fichiers Python — succès; `git diff --check` — succès; garde Cargo — 12/12 et `metadata_policy_verified`. Cargo reste avec `license=null`, `license_file=null`, `publish=[]`, `no_license_selected`, redistribution `not_assessed`, SPDX `NOASSERTION`; aucune licence n’est choisie. Les commandes et sorties sont conservées à la racine du dossier de preuves.

## Portée restante

Instance Forgejo locale jetable, un parcours (`n=1`); aucun code runtime Rust modifié, aucun AgentOS/LLM/RAG/mémoire, aucun agent hostile exécuté, aucune PR fusionnée ni validation d’un logiciel après fusion. Ce lot prouve l’émission native de deux événements de milestone PR et leur réconciliation batch vers l’API courante; il n’établit ni atomicité face à une mutation concurrente après lecture, ni gain de productivité/coût. La prochaine lacune locale reste l’autorité hôte qui choisit et protège le dépôt/ref/commit transmis au vérificateur, puis une composition sandboxée qui n’expose pas le bus D-Bus de l’hôte au candidat (CONT-374). Projet incomplet.
