# WEBHOOK-ERROR-RESOURCE-ACTIONABILITY-001 — ressource ciblée dans les erreurs

## Changement

Quand une lecture Forgejo échoue, le CLI ajoute `error.details.resource` avec `kind`, `owner`, `repository` et `number` seulement si l'enveloppe du journal est valide et que la ressource correspond exactement au propriétaire et au dépôt configurés. Le champ est ajouté aux erreurs single et batch; une ligne hors périmètre garde les détails sans identité de ressource. Les versions de réponse restent 2 (single) et 3 (batch), `details` étant déjà un objet extensible. Aucun corps HTTP amont, en-tête brut ou credential n'est recopié.

Le besoin est opérationnel : sans cette identité, un appelant ne sait pas quelle issue ou PR corriger sans relire le journal et reconstruire le contexte. Le changement n'autorise aucun accès supplémentaire.

## Preuve rouge/verte

Le scénario batch 404 a d'abord échoué avant le code produit : `details.resource.kind` était `null` au lieu de `issue` (`red.log`, code 1). Après implémentation, les deux premières tentatives vertes ont exposé un import `JournalEvent` manquant à la compilation; cet import a été ajouté. Ces erreurs de compilation sont conservées dans `green-deleted.log` et `green-out-of-scope.log`.

Les trois scénarios de processus finaux passent 1/1 chacun par `scripts/run_webhook_reconcile_cli_e2e.py`, dans Bubblewrap avec namespace réseau non partagé :

- une erreur 404 single contient l'identité de l'issue configurée;
- une erreur 404 dans un lot conserve l'identité pour les deux livraisons de cette issue, sans arrêter la ligne valide suivante;
- une erreur d'un événement hors dépôt n'expose pas son identité et ne demande qu'une lecture dans le dépôt configuré.

Les processus utilisent le vrai récepteur et le vrai binaire CLI, mais la réponse API provient de fixtures loopback synthétiques. Les tests contrôlent aussi qu'aucun corps 404 ni en-tête Authorization ne fuit dans ces cas.

## Vérifications

Avec Rust/Cargo 1.95.0, Python 3.12.3, Bubblewrap 0.9.0 et Git 2.43.0 :

- `cargo test --offline --locked --bin forgejo-reconcile-local` : 110 réussis;
- `cargo test --offline --locked --test webhook_receiver_process_e2e` : 8 réussis, 24 ignorés (les scénarios fixes doivent passer par le runner isolé);
- trois runs isolés ciblés ci-dessus : 3/3;
- `cargo fmt --all -- --check`, `py_compile`, `git diff --check` : succès;
- le garde Cargo et ses tests : 7/7. `cargo metadata --locked --offline --no-deps --format-version 1` confirme `license=null`, `license_file=null`, `publish=[]`, `no_license_selected` et `redistribution_rights=not_assessed`. Aucun choix de licence n'a été ajouté.

Les logs finaux et leur code de sortie sont à côté de ce rapport; `SHA256SUMS` couvre les entrées du dossier, tandis que `source-sha256.txt` fige les sources examinées.

## Couverture et limites

Le périmètre testé est limité au 404 sur une issue, au refus d'une requête hors dépôt, et aux versions single/batch de ce CLI. Il ne vérifie pas Forgejo en direct, une PR réelle, la fusion, un LLM, AgentOS, la résilience d'un modèle, ni un gain de temps, de tokens ou de coût. Il s'agit d'une amélioration d'actionnabilité locale dont la valeur comparative reste à mesurer.
