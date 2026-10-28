# CONTINUATION-212 — reprise du batch PR/comment après SIGKILL du receiver

## Résultat

Une instance jetable Forgejo `17.0.0+gitea-1.22.0` a émis nativement les événements `pull_request/review_requested` et `issue_comment/created` (`is_pull=true`) pour la même PR ouverte. Les deux tâches Forgejo ont reçu HTTP 202 avant l’interruption. Le readback local en lecture seule de `data/forgejo.db` confirme leurs deux UUID et statuts; Forgejo nomme ces tâches internes `pull_request_review_request` et `pull_request_comment`, distinctement des noms canoniques du receiver.

Après ces accusés, le processus receiver précis du harnais reçoit `SIGKILL` (`returncode=-9`). Son listener disparaît; un nouveau processus est lancé sur la même adresse, avec le même secret privé et le même journal. Les deux enregistrements, leur ordre et leurs IDs sont relus; le SHA-256 du journal reste `93cf415cdc74563ace8da8a797040e4425102e8d1fa97e8b28c2a5a77a45b3b5` avant et après.

Le CLI Rust lit ensuite le batch depuis ce journal repris. Il retourne `current_state_read`, avec exactement deux GET Forgejo réussis : la PR et le commentaire par ID. Le parent, la base/tête et la liste reviewer d’équipe restent cohérents; le commentaire courant porte `body_trust=untrusted_user_content`. La réconciliation ne modifie pas l’inbox. La tentative de POST commentaire avec le token read-only est refusée 403 sans effet. Les deux lignes de tâche filtrées (sans payload brut) sont dans `forgejo-hook-task-acks.json`. À la fin, le receiver redémarré et Forgejo sortent proprement; aucun listener ne subsiste.

## Run reproductible

```sh
python3 scripts/run_forgejo_live_org_team_reviewer_readback.py \
  --forgejo-binary /tmp/forgejo-17.0.0 \
  --state-dir /tmp/forgejo-team-review-cont212-state-002 \
  --evidence-dir evidence/continuation-212-live-pr-comment-hard-restart-002 \
  --probe-pr-comment-batch \
  --kill-receiver-before-reconcile
```

Le runner construit les binaires Rust dans Bubblewrap 0.9.0 avec le réseau et les PID non partagés, sources/sysroot en lecture seule et Cargo offline/locked. Forgejo lui-même est exécuté localement avec endpoints loopback. Le test utilise le readback SQLite Forgejo en mode lecture seule pour établir les réponses HTTP 202 avant SIGKILL; ce schéma interne n’est pas une API publique stable.

Forgejo version `17.0.0+gitea-1.22.0`, SHA-256 binaire `e5ec437cd6357e8d1b4580cb47ab1daa58a32b33971a63db63446a535d33adb7`; le commit exact de build reste inconnu. Python 3.12.3, Rust/Cargo 1.95.0. Run final `n=1`, état privé `/tmp/forgejo-team-review-cont212-state-002`.

## Tests et essais conservés

Les 2 tests du validateur de frontière SIGKILL/journal passent avec les autres tests du module ciblé (`24/24`). Le test-first échouait avant l’ajout du validateur; le module a aussi révélé que le nom interne de tâche Forgejo diffère du nom d’événement du receiver. L’implémentation distingue maintenant les deux sans projeter le payload SQLite. Le garde Cargo et ses 12 tests passent; le reçu conserve `license=null`, `license_file=null`, `publish=[]`, `no_license_selected`, droits `not_assessed` et `NOASSERTION`. `py_compile`, le validateur indépendant du résultat et `git diff --check` passent.

L’essai `attempt-001-validator-mapping-failure.log` a échoué uniquement parce que le validator assimilait le nom interne de tâche à l’événement canonique. La lecture SQLite après arrêt montre que les deux tâches avaient en fait été acquittées 202; le run corrigé utilise une nouvelle instance et un nouveau répertoire d’état. Le premier test unit de l’implémentation avait également une fixture incomplète (elle omettait le champ `event`); cette fixture a été corrigée avant le résultat final.

## Portée et limites

Cette preuve vérifie une reprise réelle après arrêt brutal du receiver, après accusé HTTP 202, puis la réconciliation API du batch PR/comment sur le journal conservé. Elle ne prouve pas une panne d’alimentation, une corruption de disque, une durabilité multi-hôte ou une garantie générale de tout système de fichiers. La lecture de `hook_task` est liée au schéma interne de Forgejo 17.0.0. C’est un adapter de test local; aucun agent LLM, AgentOS, RAG, mémoire, verdict d’acceptation, PR logicielle candidate, merge ou benchmark n’est exécuté. Aucun coût fournisseur, service externe, publication, production ou code Forgejo/GitSpace copié.

Les droits de redistribution restent `not_assessed` et aucune licence produit n’est choisie. Les critères de fin complets restent ouverts.
