# Observation rouge initiale

Commande exécutée avant l’implémentation :

```text
cargo test --offline --locked --test webhook_handler_contract forgejo_v17_issue_comment_wire_event_binds_issue_and_pull_request_comments -- --exact --nocapture
```

Résultat : code de sortie 101; le test échoue à la première livraison. Le récepteur retourne HTTP 422, code `unsupported_event`, action demandée : configurer les événements pris en charge. La requête est construite avec une HMAC correcte du corps; le contrôle d’événement se produit avant sa vérification côté récepteur. Après l’ajout de `issue_comment` et sa liaison par le payload, le test est vert dans `handler-contract.log`.

Cette observation est la sortie du premier essai de ce lot; la commande et le comportement sont décrits ici, tandis que les sorties finales sont conservées dans les logs.
