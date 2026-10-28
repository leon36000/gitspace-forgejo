# WEBHOOK-NATIVE-REPLAY-005 — hook inactif, tâches et comptage HTTP

Date : 24 octobre 2026. Instance Forgejo v17 jetable en loopback, dépôt privé et receiver local. Ce run distingue le succès HTTP du POST de replay, la création d'une tâche Forgejo et une livraison effectivement envoyée.

## Commande exacte

```sh
PYTHONDONTWRITEBYTECODE=1 python3 scripts/run_forgejo_live_webhook_native_replay_e2e.py \
  --forgejo-binary /tmp/forgejo-17.0.0 \
  --repo-root /home/pc6/Documents/Codex/2026-10-21-mission-construire-une-forge-con-ue/.runtime/webhook-005-workspace \
  --state-dir /tmp/forgejo-native-replay-005-state \
  --evidence-dir /home/pc6/Documents/Codex/2026-10-21-mission-construire-une-forge-con-ue/.runtime/webhook-005-workspace/evidence/webhook-native-replay-005
```

Le runner compile le vrai receiver Rust hors ligne/verrouillé sous Bubblewrap. Forgejo, le proxy compteur et le receiver ne servent que des adresses loopback. Le proxy transmet le corps inchangé et ne garde que méthode, chemin, identifiant, type d'événement, taille/hash du corps et statut amont; il ne stocke ni en-têtes d'authentification/signature ni corps brut. SQLite est lu en mode read-only pour les `hook_task` du Forgejo v17.

## Résultat mesuré

- Binaire `17.0.0+gitea-1.22.0`, SHA-256 `e5ec437cd6357e8d1b4580cb47ab1daa58a32b33971a63db63446a535d33adb7`. Le code Forgejo officiel v17.0.0 examiné est au commit `0d791421f85ca13a63b665053d81ea765fbbc571`; le commit de build du binaire local n'est pas établi.
- L'événement initial `issues/opened` obtient 401 à cause d'un secret volontairement incorrect. Le proxy observe une requête. Pendant l'observation d'une seconde, il ne voit pas de nouvelle tentative et Forgejo conserve une seule tâche.
- Les refus sans session (303 vers login), cross-origin (403) et UUID absent (404) ne créent aucune tâche et n'atteignent pas le proxy webhook.
- Deux POST authentifiés sur le même UUID initial renvoient chacun 303 et créent des tasks distincts. Le proxy observe les UUID dans l'ordre : échec initial 401, puis deux replays 202. Les SHA-256 des corps bruts sont identiques (`6765db97a023fb6482897889e5178d5c848aa49d76dca2985ca5b3e8f3e61df8`); les reçus receiver lient les deux UUID aux payloads JSON sémantiquement identiques, séquences 1 et 2.
- Après readback `active=false`, un nouveau POST sur l'UUID initial renvoie tout de même 303 et Forgejo crée une quatrième tâche avec UUID neuf. Le worker la marque `is_delivered=true`, `is_succeed=false`, `response_status=0`; le proxy reste à trois requêtes et le journal receiver à deux événements. Le 303 signifie ici que le replay a été accepté/enfilé, pas qu'une livraison a eu lieu. Cette observation concorde avec le code v17 : le handler ne refuse pas le hook inactif; le worker marque la tâche tentée puis la saute avant l'appel HTTP.
- Le hook est relu inactif; le proxy, receiver et Forgejo sont arrêtés; leurs listeners vérifiés sont absents. L'état privé `/tmp/forgejo-native-replay-005-state` est conservé.

## Vérifications et provenance

- `PYTHONDONTWRITEBYTECODE=1 python3 -m unittest -v tests.test_forgejo_webhook_native_replay_e2e` : 7 réussis, 0 échec. Le test du replay inactif refuse notamment un hook actif, une réponse 403, un statut HTTP 202 de livraison et une requête proxy/journal supplémentaire.
- `python3 -m py_compile scripts/run_forgejo_live_webhook_native_replay_e2e.py tests/test_forgejo_webhook_native_replay_e2e.py` passe.
- Le runner et les tests sont identifiés dans `provenance.json`; les journaux, reçus et hashes sont couverts par `SHA256SUMS`.
- Sources primaires au tag v17.0.0 : `routers/web/repo/setting/webhook.go:461–480` accepte et redirige après le replay; `models/webhook/hooktask.go:153–170` copie le payload/type/version dans une task à UUID neuf; `services/webhook/deliver.go:97–105,135–144` marque la tâche tentée, puis saute l'appel HTTP si `!w.IsActive`.

## Limites

Run `n=1`, admin local, et lecture de la table SQLite interne Forgejo v17. Aucun test Go upstream; le binaire n'est pas relié au commit source audité. Le proxy vérifie les requêtes HTTP reçues par l'URL configurée, mais le service complet n'est pas isolé contre un processus du même UID. Ce run n'ajoute pas le parcours complet demande→exigences→agent→PR→gate→fusion, n'utilise pas de LLM/AgentOS/Forge Code/RAG/mémoire et ne mesure pas de gain. Il démontre le comportement de cette instance/version, pas une garantie générale de toutes les versions Forgejo.
