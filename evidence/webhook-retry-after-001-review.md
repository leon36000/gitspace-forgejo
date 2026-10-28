# Revue indépendante — WEBHOOK-RETRY-AFTER-001

Date : 24 octobre 2026. Revue statique en lecture seule par un sous-agent Codex; l’interface ne permet pas de sélectionner Sonnet 5.5.

## Conclusion

Aucun défaut matériel actionnable trouvé dans la gestion de `Retry-After`, le parseur de date ou la borne de délai.

La revue a vérifié `parse_retry_after` et `parse_http_date_epoch_seconds` dans `src/bin/forgejo-reconcile-local.rs:1111`, le contrôle de l’échéance dans `read_current` autour de la ligne 1046, et les tests du budget dépassé, des trois formats de date, du nombre maximal d’essais et du délai partagé (`:8022`, `:8055`, `:8128`). Les exemples de dates aboutissent au même instant; les retries restent limités par le nombre d’essais et le budget monotone.

La revue n’a exécuté aucun test. Elle ne démontre pas l’interopérabilité avec plusieurs versions de curl ni tous les cas limites des dates HTTP. Les vérifications exécutées séparément sont consignées dans [le rapport local](webhook-retry-after-001-local-run.md).

## Addendum WEBHOOK-RETRY-AFTER-002

Revue statique par le même sous-agent Codex : aucun défaut actionnable établi dans `start_cli_http_fixture_with_headers` ou le nouveau test de processus. La fixture compte les requêtes; le test attend la fin du CLI avant de joindre la fixture et recoupe le compte avec `upstream_read_attempts == 1`, le statut HTTP et le verdict inconclusif. Le runner nettoie l’environnement et exécute le test dans `bwrap --unshare-net`, ce qui isole le port loopback 3130 du réseau hôte.

Cette revue n’a exécuté aucun test. Sa conclusion ne remplace pas le run `python3 scripts/run_webhook_reconcile_cli_e2e.py` enregistré dans `webhook-retry-after-001-process-e2e.log`.


## Addendum WEBHOOK-RETRY-AFTER-003

La première lecture statique a identifié un faux positif possible : une date lointaine rejetée comme invalide pouvait aussi mener à une seule requête. Le test a été modifié pour couvrir séparément une date invalide, une date future au-delà du budget et une date future valide dans le budget. La dernière revue confirme que la date valide est créée à la première réponse 429 et que l’intervalle réponse→connexion suivante mesure le délai réellement observé; l’assertion `>= 2 s` distingue le comportement du backoff court. Le runner inclut ce test dans son exécution isolée sous `bwrap --unshare-net`.

Conclusion finale : aucun défaut matériel actionnable établi dans le test ou le runner. Revue statique seulement : aucun test n’a été lancé par le réviseur. Les résultats exécutés sont consignés dans [le rapport local](webhook-retry-after-001-local-run.md) et les journaux WEBHOOK-RETRY-AFTER-003.


## Addendum WEBHOOK-OUT-OF-ORDER-001

Revue statique indépendante après correction des faux positifs successifs : le scénario hors ordre compare bien le readback canonique distinct des deux payloads. Le harnais d’échéance observe chaque GET après réception des en-têtes complets, exige un temps CLI d’au moins 29 s, borne les retries du huitième numéro, et garde le fixture actif jusqu’à la fin/arrêt du groupe CLI avant de drainer les connexions en file. La revue ne trouve plus de défaut matériel; seule réserve, l’horloge d’assertion démarre juste avant le spawn et est donc légèrement conservatrice par rapport au départ interne du budget. Le réviseur n’a exécuté aucun test. Les commandes et résultats sont dans [le rapport local](webhook-retry-after-001-local-run.md), avec logs et SHA-256 `webhook-out-of-order-001-*`.
