# FORGEJO-NATIVE-PR-BATCH-RECONCILE-001 — résultat local

Date : 25 octobre 2026. Worktree : `local/WEBHOOK-005-canonical-links`, HEAD observé `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`; arbre déjà fortement modifié avant ce lot. Aucun commit ou staging effectué. Le driver et ses tests sont des sources locales de l’arbre courant; leurs empreintes d’exécution figurent dans `provenance.json`.

## Changement et parcours

Le driver accepte maintenant `--batch-pull-request-events`. Il rassemble les livraisons PR dans leur ordre observé, transmet les IDs uniques au CLI Rust en une seule invocation, passe le credential de lecture par pipe/FD, exige sa consommation et refuse toute sortie qui l’exposerait. Le validateur vérifie le schéma, le verdict, le compteur agrégé d’une lecture amont, le nombre et l’ordre des résultats, l’identité livraison/résultat, le dépôt, le numéro de PR et les SHA d’état courant. Les reconstructions individuelles restent disponibles comme artefacts; le batch est archivé séparément.

Commande exécutée :

```text
python3 scripts/run_forgejo_live_webhook_reconcile_emitted.py --forgejo-binary /tmp/forgejo-17.0.0 --repo-root . --state-dir /tmp/forgejo-native-pr-batch-001-state --evidence-dir evidence/forgejo-native-pr-batch-reconcile-001 --request-changes-review --synchronize-pull-request --batch-pull-request-events
```

Une instance Forgejo jetable v17.0.0 a émis neuf tâches de webhook : quatre événements issue et cinq événements de PR (`opened`, `synchronized`, commentaire de conversation, revue `REQUEST_CHANGES`, puis fermeture non fusionnée). Les neuf tâches ont reçu HTTP 202 et sont liées aux livraisons du journal. Le vrai CLI réconcilie les cinq événements PR en un batch : cinq résultats ordonnés, une ressource PR, et `upstream_read_attempts=1` au niveau du batch. Le compteur est celui du CLI, pas une capture réseau indépendante. L’API Forgejo confirme ensuite la PR #2 `closed`, `merged=false`, les SHA courants et la fermeture non fusionnée. Le replay après redémarrage SIGKILL du récepteur ne modifie ni le nombre ni les octets du journal. Le credential invalide reste `inconclusive/upstream_unauthorized`. La hook est désactivée par readback; Forgejo et le récepteur sont arrêtés, leurs ports n’ont plus de listener.

## Vérifications

- Forgejo local : version `17.0.0+gitea-1.22.0`, binaire SHA-256 `e5ec437cd6357e8d1b4580cb47ab1daa58a32b33971a63db63446a535d33adb7`. Le dépôt source scratch est au commit officiel `0d791421f85ca13a63b665053d81ea765fbbc571`, mais il contient des modifications POC non liées; le commit exact de build du binaire n’est pas établi.
- Le parcours a reconstruit les binaires Rust avec Cargo 1.95.0/rustc 1.95.0 sous Bubblewrap 0.9.0, en mode offline/locked et réseau isolé. Hashes des exécutables et des sources utilisées : `provenance.json`.
- `python3 -m unittest tests.test_forgejo_emitted_webhook_e2e` : 71/71.
- `python3 -m py_compile scripts/run_forgejo_live_webhook_reconcile_emitted.py tests/test_forgejo_emitted_webhook_e2e.py` : succès.
- Garde Cargo : 7/7; `license=null`, `license_file=null`, `publish=[]`, `no_license_selected`, droits `not_assessed`.
- `git diff --check` : succès.

Le lot ne change pas le code Rust et ne relance donc pas la suite Cargo complète. Les journaux, lectures API, tâches Forgejo, événements, sortie batch, readbacks de nettoyage, hashes et manifeste sont conservés dans ce dossier.

Après le run live, le validateur a été renforcé pour exiger un type `int` exact sur le numéro de PR dans la sélection et dans chaque résultat, et un test refuse le cas booléen. Le run live consigne le hash du code effectivement exécuté dans `provenance.json`; la source actuelle et le test actualisé ont ensuite repassé les 71 tests et `py_compile`, consignés dans `python-tests-final.log`, `pycompile-final.log` et `source-sha256-final.txt`. La composition Forgejo n’a pas été rejouée après ce durcissement de type; les nouvelles assertions unitaires portent sur ce changement précis.

## Limites

Un run local (`n=1`) mesure l’intégration du batch et son compteur applicatif, pas une amélioration générale ou un coût réellement moindre en production. Le run ne fusionne pas cette PR; une composition antérieure documente séparément la fusion contrôlée et le readback post-merge. Aucun LLM/AgentOS/RAG/mémoire, CI conventionnelle, service externe, coût modèle ou benchmark n’a été utilisé. Forgejo, son récepteur et le CLI tournent sous le même compte hôte; ce parcours n’établit pas d’isolation multi-tenant. Aucun code Forgejo ni GitSpace n’a été copié au produit; la licence produit reste non choisie.

Suite locale : poser explicitement les invariants de publication LFS à travers les frontières Git/DB et comparer, dans le scratch Forgejo existant, un témoin durable à un verrou/fence couvrant les writers. Les résultats existants POC-060 et POC-064 montrent deux interleavings opposés; ils ne décident pas encore la sémantique de récupération. Maintenir le patch non adopté et ne pas annoncer le benchmark général avant un plafond modèle fiable et autorisé.
