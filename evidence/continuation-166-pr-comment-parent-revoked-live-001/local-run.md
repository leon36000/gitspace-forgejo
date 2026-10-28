# CONTINUATION-166-RUN-001 — échec du harnais avant révocation

Date : 27 octobre 2026 UTC. Worktree local fondé sur HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`, déjà fortement modifié. Aucune remise à zéro, aucun commit, aucun déploiement ou changement de production.

Commande lancée :

```sh
python3 scripts/run_forgejo_live_webhook_reconcile_emitted.py \
  --forgejo-binary /tmp/forgejo-17.0.0 \
  --repo-root . \
  --state-dir /tmp/forgejo-pr-comment-parent-revoke-live-166 \
  --evidence-dir evidence/continuation-166-pr-comment-parent-revoked-live-001 \
  --delete-pull-request-comment \
  --revoke-pull-request-comment-reader-before-reconcile
```

Le run a créé l’instance de test, reçu les événements et s’est arrêté à `revoke_comment_reader_and_confirm_parent_visibility` avec `NameError: name 'quote' is not defined`. La construction de la route d’API a échoué avant l’appel de révocation; aucune conclusion sur la visibilité du parent n’est tirée de cet essai. Le récepteur et Forgejo ont été arrêtés; le reçu `port-cleanup-readback.json` confirme l’absence de listeners et de défaut de nettoyage. Le défaut du harnais a été corrigé en important `urllib.parse.quote`; le run neuf passe dans le reçu [CONTINUATION-166](../continuation-166-pr-comment-parent-revoked-live-002/local-run.md).

Le binaire local avait SHA-256 `e5ec437cd6357e8d1b4580cb47ab1daa58a32b33971a63db63446a535d33adb7`. L’échec est conservé comme essai rouge; le runner n’a pas écrit `provenance.json`, donc le hash exact du script à cet appel n’est pas établi. Les octets de cet essai ne doivent pas être confondus avec ceux validés dans le run 002. Aucun code amont copié, aucune dépense, aucun secret externe et aucun processus préexistant inspecté ou contrôlé.
