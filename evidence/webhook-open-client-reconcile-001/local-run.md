# WEBHOOK-OPEN-CLIENT-RECONCILE-001 — traiter une requête HTTP complète avant EOF client

Date : 2026-10-24. Révision de départ : `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0` (`local/WEBHOOK-005-canonical-links`); worktree déjà modifié. Les fichiers de preuve décrivent uniquement le lot ajouté dans ce worktree.

## Ce qui est vérifié

`actual_reconcile_cli_process_accepts_complete_request_before_client_half_close` démarre le binaire local `forgejo-webhook-local`, envoie une requête HTTP/1.1 HMAC-synthétique dont le corps complet est borné par `Content-Length`, annonce `Connection: keep-alive` et garde ouvert le côté écriture TCP. Le client ne fait pas `shutdown(Write)`. Le receiver répond néanmoins `202`, ferme sa réponse selon l’en-tête et journalise exactement une livraison avec sa clé d’idempotence.

Le test démarre ensuite le vrai binaire `forgejo-reconcile-local` sur cette livraison. Le seul upstream est une fixture HTTP privée liée à la boucle locale et au port allowlisté, sous `bwrap --unshare-net`; elle reçoit un unique `GET` non authentifié. Le résultat JSON v2 est `current_state_read` et reflète l’état courant `open`, même si le payload signé disait `closed`. Le journal reste octet pour octet identique après la réconciliation.

Commande ciblée :

```text
python3 scripts/run_webhook_reconcile_cli_e2e.py --scenario actual_reconcile_cli_process_accepts_complete_request_before_client_half_close
```

Résultat : 1/1 scénario réussi, 20 filtrés, dans la fixture réseau isolée. La commande de compilation associée utilise `cargo test --offline --locked --test webhook_receiver_process_e2e --no-run`.

Suite complète exécutée après correction de l’attente de test périmée :

```text
cargo test --offline --locked --all-targets
```

Résultat du journal : 34 groupes de tests, 434 réussites, 0 échec, 27 ignorés. Cela couvre les cibles Rust locales de ce worktree, mais les tests ignorés exigent leur runner ou leur environnement dédié; ce compte ne les inclut pas.

## Échec et correction de harnais

Le premier essai isolé interprétait à tort le résultat single comme le schéma batch `results[]`. Le CLI single utilise les champs de premier niveau `delivery_id`, `resource` et `canonical_state`; cette assertion du test a été corrigée et l’essai isolé est ensuite passé. Le journal rouge est conservé dans `initial-single-schema-assertion-failure.log`.

La première suite `--all-targets` a aussi révélé qu’un test du délai de verrouillage attendait l’ancien message stderr non structuré et une sortie vide, alors que le contrat actuel renvoie sur stdout le JSON v2 `inconclusive` / `delivery_not_found`, sans tentative upstream. Le test a été ajusté pour vérifier ces champs et préserver l’invariant de libération du verrou dans la fenêtre d’attente. Reproduction rouge, run ciblé vert (3/3) et première suite en échec conservés séparément. Aucun changement de logique de production n’a été effectué dans ce lot.

## Couverture et limites

C’est une intégration locale des deux binaires et du journal avec des fixtures synthétiques. Le serveur API est une fixture, pas une instance Forgejo; cela ne prouve pas l’API Forgejo, une PR, la vérification indépendante, une fusion, un déploiement ou un parcours LLM. Le test contrôle un socket local et un client qui garde son côté écriture ouvert; il ne constitue ni un test de charge ni une mesure de gain utilisateur. L’acceptation receiver et le readback du CLI sont exécutés dans le même test, pas par un agent LLM.

Les logs rouges et verts, la version des outils, les empreintes des sources et le manifeste de cette preuve se trouvent dans ce répertoire. `SHA256SUMS` couvre les fichiers du répertoire sauf lui-même.

Le contrôle du périmètre de licence Cargo passe aussi 5/5 : le readback conserve `license=null`, `license_file=null`, `publish=[]`, décision `unselected`, droits `not_assessed`. Aucun choix de licence n’a été fait. Les vérifications de formatage, compilation Python, diff-check et espaces de fin passent; résultats exacts dans `checks.log`.
