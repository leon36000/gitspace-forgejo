# CONTINUATION-016 — erreurs de payload actionnables

Date de travail : 25 octobre 2026. Dépôt produit : `agent-forge-core`, version Cargo 0.1.0, checkout de travail `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0` au début de la tranche, fortement modifié. Les hashes des sources finales sont dans `source-sha256.txt`; les sorties et codes des commandes sont conservés dans ce dossier.

## Lacune et changement

Une entrée d’événement contenant un champ réservé à un autre type d’événement était refusée comme `invalid_envelope`, mais ni le CLI ni le receiver HTTP ne disaient quel champ était en cause. L’opérateur ne pouvait donc pas distinguer cette erreur de forme des autres enveloppes invalides.

Le reconciler identifie maintenant les champs événementiels interdits avec `ReconcileError::UnexpectedEventPayloadField`. L’ingestion transforme ce cas en `WebhookError::InvalidEventPayloadField`. Le receiver retourne toujours HTTP 400 avec le code `event_payload_invalid`, mais ajoute `error.details.field_path` et une action demandant d’inspecter la source puis de préserver le journal append-only. Le CLI conserve `inconclusive/invalid_envelope` et `retryable=false`, et ajoute le même détail. `/forgejo/capabilities` décrit les champs d’erreur requis et optionnels et le format de `payload.<field>`. Les chemins sont relatifs au payload signé. Les verdicts, codes et refus avant les effets restent inchangés; un élément invalide n’arrête pas le traitement des autres livraisons du lot.

## Test-first et correction observée

Trois assertions ont été ajoutées avant l’implémentation :

1. Le vrai CLI doit indiquer `payload.review` pour une entrée de journal rejouée contenant `review: null`.
2. Le vrai processus receiver doit indiquer le même chemin quand un corps signé `issues` contient ce membre.
3. Le endpoint de capacités doit annoncer le contrat détaillé.

Les trois tests ont échoué avant correction (`red.log`, `http-red.log`, `capabilities-red.log`; codes enregistrés dans les fichiers `.exit`) parce que `details.field_path` ou `error_contract` était absent. Après correction, les tests du receiver HTTP et des capacités passent (`receiver-process.log`, `capabilities-green.log`); les deux scénarios de processus CLI passent dans le runner allowlisté et sandboxé (`cli-single.log`, `cli-batch.log`). Le scénario de lot confirme qu’un refus reste local à sa livraison et que l’événement valide suivant reçoit un seul GET de fixture, sans mutation du journal.

## Vérifications

- `cargo test --locked --offline --all-targets` : **536 réussis, 0 échec, 76 ignorés, 34 groupes** (`all-targets-final.log`). Les ignorés incluent des essais qui exigent le runner processus dédié ou une infrastructure de test distincte.
- `python3 scripts/run_webhook_reconcile_cli_e2e.py --scenario actual_reconcile_cli_process_rejects_legacy_issue_review_member_before_api_read` : 1/1, Bubblewrap avec namespaces réseau et PID isolés.
- `python3 scripts/run_webhook_reconcile_cli_e2e.py --scenario actual_reconcile_cli_process_continues_batch_after_legacy_issue_review_member` : 1/1, mêmes frontières.
- Test ciblé du vrai receiver, test ciblé de découverte des capacités, contrats webhook et contrats de réconciliation : sorties conservées dans les fichiers de ce dossier.
- `cargo fmt --all -- --check`, `git diff --check`, garde de licence Cargo et readback `cargo metadata --no-deps --format-version 1` : sorties conservées dans les fichiers de ce dossier.

## Environnement et droits

Les versions d’outils sont dans `tool-versions.txt`. Le readback Cargo confirme `license=null`, `license_file=null`, `publish=[]`, décision `no_license_selected`; les droits de redistribution restent `not_assessed` et le garde SPDX retourne `NOASSERTION`. Aucun choix de licence n’est introduit.

Ce lot n’a pas contacté d’instance Forgejo, d’AgentOS, de modèle LLM ou de service externe; aucun coût ou secret n’a été engagé. Les tests utilisent des fixtures loopback ou le journal temporaire. Le test du processus HTTP direct tourne sous le même UID et n’est pas un sandbox de sécurité.

## Limites et risque d’intégration

Les cas ciblés couvrent le champ interdit `review` et les autres deny-lists déjà présentes dans le validateur; ils ne rendent pas tous les champs inconnus actionnables et ne démontrent pas la compatibilité de chaque payload historique. Le journal ne conserve pas le HMAC d’origine des entrées; les cas historiques sont synthétiques. L’ajout de variantes à des enums Rust publics peut casser des consumers externes qui font un `match` exhaustif; aucun consumer externe n’a été examiné.

Cette amélioration clarifie un refus; elle ne prouve pas la correction du code soumis, une fusion sûre, l’usage d’un agent LLM ou un gain comparatif. Le projet global reste incomplet.
