# FORGEJO-PR-MERGED-WEBHOOK-001 — fusion locale suivie du webhook natif et de la réconciliation

Date : 25 octobre 2026. Worktree produit `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`, fortement modifié et préservé.

## Résultat observé

Le runner a utilisé Forgejo v17.0.0 sur `127.0.0.1:3130`, base SQLite et dépôt privé jetables. Une PR locale (#2) a été créée avec un fichier candidat. L’API de fusion a reçu `Do=merge` et le `head_commit_id` observé à l’ouverture. Après le `POST`, le runner a relu la PR : état `closed`, `merged=true`, `merge_commit_sha=84043c9c8702d0a247f2a345ec0f162d9c3b44dd`.

Forgejo a émis l’événement natif `pull_request`/`closed`. La tâche Forgejo liée à cette livraison a réussi en HTTP 202. Son corps indiquait `merged=true` et le même `merge_commit_sha`. Le vrai receiver a ajouté l’événement à son inbox durable. Le vrai CLI `forgejo-reconcile-local`, utilisant le lecteur non-admin en lecture seule, a fait un seul GET de la PR et retourné `current_state_read`, `closed`, `merged=true`, le même SHA de fusion, ainsi que les têtes courante et candidate. Le journal n’a pas changé pendant la réconciliation.

Le runner a ensuite relu le fichier candidat sur `main` et comparé ses octets, puis confirmé que la tête de `main`, la lecture de la PR, le webhook et la réconciliation désignaient le même commit de fusion. L’API de fusion n’a pas fourni un corps JSON : son HTTP 200 est confirmé par la relecture de PR. Le hook a été désactivé avec readback; Forgejo, le receiver et leurs ports sont arrêtés.

## Premier échec et correction

La tentative 001 a fusionné la PR, mais le harnais s’est arrêté à `deliver_closed_pull_request_event` car il supposait que le `POST /pulls/{n}/merge` renverrait un objet JSON contenant `merged` et `sha`. La lecture de la base SQLite jetable conservée dans `failure-observation.json` montre la PR fusionnée et la tâche webhook créée, encore en attente quand le runner s’est arrêté. Le code audité de `routers/api/v1/repo/pull.go::MergePullRequest` finit le chemin réussi par `ctx.Status(200)` sans corps JSON (ligne 1082). Le harnais corrigé traite le `POST` comme un acquittement HTTP et s’appuie sur un GET de PR avant d’attendre la livraison native. La tentative échouée reste préservée dans `../forgejo-pr-merge-webhook-001/`.

## Provenance et vérifications

- Binaire local : `17.0.0+gitea-1.22.0`, SHA-256 `e5ec437cd6357e8d1b4580cb47ab1daa58a32b33971a63db63446a535d33adb7`; son commit de build exact n’est pas établi.
- Source Forgejo officielle examinée : v17.0.0, commit `0d791421f85ca13a63b665053d81ea765fbbc571`. Le scratch `/tmp/forgejo-v17-base-cas-poc` est globalement modifié par des POC; les fichiers source ciblés et leurs hashes sont consignés dans `upstream-source-sha256.txt`.
- Les points contrôlés sont `services/webhook/notifier.go::MergePullRequest` (action `closed`), `modules/structs/hook.go::PullRequestPayload`, `modules/structs/pull.go::PullRequest.MergedCommitID` (`merge_commit_sha`) et le handler REST `MergePullRequest`.
- Build des binaires Rust `--offline --locked` dans Bubblewrap avec réseau et PID isolés, environnement effacé et sources montées en lecture seule; détail dans `provenance.json` et `cargo-build.log`.
- Suite Python du driver : 59/59; tests du parseur PR : 8/8; `py_compile`, `cargo fmt --all -- --check`, `git diff --check` réussissent. Une assertion négative de parseur a été ajoutée après le parcours live; elle ne modifie pas le runner. Le run live reste lié aux empreintes de son `provenance.json` (source de test au run : `1d0d3b533e4fc2742abce01c022aaca1a6933fe12fa88dff13cac46b54216748`; source de test après ajout de l’assertion : `ef62f19b0a30f6a9d994738f58ac492960019c1459a1bfba7504d6f011759f7f`). Le manifeste local passe 25/25.

Commande du parcours réussi :

```sh
python3 scripts/run_forgejo_live_webhook_reconcile_emitted.py \
  --forgejo-binary /tmp/forgejo-17.0.0 \
  --state-dir /tmp/forgejo-pr-merge-webhook-20261025-002 \
  --evidence-dir "$PWD/evidence/forgejo-pr-merge-webhook-002" \
  --merge-pull-request
```

## Portée et limites

Un run, une PR, une instance locale. Le propriétaire de l’instance de test a déclenché directement la fusion; ce parcours ne vérifie pas la politique de branches, une approbation indépendante, le gate de fusion du produit ou une course sur la base. Le lot établit le lien live fusion REST → événement Forgejo → inbox → état canonique et présence du fichier fusionné. Le parcours protégé avait déjà été composé dans le run MISSION-012-COMPOSE-062 avec les événements PR natifs; la preuve 062 a ensuite été complétée par un test en checkout frais du commit exact de fusion. Ce lot 002 reste une expérience distincte, et ajoute la lecture live du champ `merge_commit_sha` dans l’événement natif et le reconciler; il ne constitue pas un benchmark ni une nouvelle preuve de gate. Aucun agent réel ni CI conventionnelle n’a été appelé; tokens modèle et coût modèle : 0 parce qu’aucun appel modèle n’a eu lieu. Une réussite `n=1` ne prouve aucun gain général. Aucun code Forgejo ou GitSpace n’a été copié et aucun choix de licence n’a été fait.

La mission reste incomplète. Le composeur MISSION-012-COMPOSE-064 exige maintenant explicitement que le SHA de fusion du webhook, des reçus de réconciliation et de l’état canonique corresponde au résultat de l’autorité; ce comportement a été exercé dans Forgejo local et le commit exact a passé un contrôle après fusion en clone frais. Voir [le rapport composé](../mission-012-live-agent-webhook-local-adapter-064/local-run.md). Ensuite, poursuivre les refus manquants et la mesure comparative seulement si un agent déjà autorisé et un plafond de coût fiable sont disponibles.
