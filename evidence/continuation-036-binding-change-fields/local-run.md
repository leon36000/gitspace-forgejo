# CONTINUATION-036 — champs de liaison PR modifiés explicitement

Date : 26 octobre 2026. Workspace : `.runtime/webhook-005-workspace`. Branche observée : `local/WEBHOOK-005-canonical-links`. HEAD observé : `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`; le worktree était fortement modifié et reste non commité.

## Lacune et correction

Le snapshot lié à une PR ne rendait pas visible quel élément de la liaison avait changé entre les deux lectures. Il retourne maintenant `pull_request_binding_changed_fields`, tableau ordonné selon `number`, `state`, `merged`, `base_branch`, `base_sha`, `head_sha`; la découverte des capacités annonce cette liste et sa sémantique. La preuve rouge `base-drift-red.log` montre que le cas où seul `base_sha` changeait ne rapportait aucun champ; `capabilities-red.log` montre la propriété absente de la découverte.

La revue du nouveau cas a aussi séparé deux faits jusqu’alors confondus : `pull_request_head_stable_between_reads` ne décrit que l’égalité du SHA de tête, alors que `pull_request_binding_stable_between_reads` décrit l’égalité de tous les champs listés. Un déplacement de base avec tête identique retourne donc `observation=inconclusive`, `changed_fields=["base_sha"]`, tête stable `true`, liaison stable `false` et `current_pr_head_checked=true`. L’action demande de relire le snapshot sans laisser entendre que le SHA de tête a changé.

## Vérifications

Toolchain et commandes exactes : [tool-versions.txt](tool-versions.txt), [commands.txt](commands.txt). Les résultats compilés sont conservés dans les logs liés par le manifeste.

- Les tests unitaires `tests::pull_request_status_snapshot_` passent 5/5 : liaison stable, dérive de tête, dérive de base avec tête stable, statuts absents et pagination incomplète.
- Le test processus du vrai CLI passe 1/1 sous Bubblewrap avec namespaces réseau et PID isolés. La fixture loopback vérifie les cas tête stable, tête modifiée et aucun statut; le scénario de dérive de base est au niveau adaptateur/unitaire, pas processus.
- Le test de découverte des capacités passe 1/1 et la suite de ce fichier passe 3/3; deux tests port fixe restent ignorés par défaut et sont lancés avec le filtre explicite lorsqu’ils sont nécessaires.
- La compilation de la cible d’intégration, Rustfmt, `git diff --check` et le garde de licence Cargo passent. Clippy passe avec les quatre allowances de lint explicites consignées dans la commande. Clippy strict échoue sur 12 diagnostics dans des modules hors de cette tranche; le log complet est conservé.
- Cargo reste `license=null`, `license_file=null`, `publish=[]`, `no_license_selected`, `redistribution_rights=not_assessed`.

## Portée et limites

La fixture est un serveur HTTP synthétique loopback et le processus testé est le vrai CLI local. Aucun Forgejo actif, agent LLM, AgentOS, credential réel, serveur externe, secret, dépense nouvelle, publication, production, création de PR ou fusion n’intervient. Les changements visibles ne couvrent que les six champs énumérés et ne rendent pas la lecture Forgejo atomique; une modification suivie d’un retour à la même valeur entre les lectures peut échapper à ce contrôle. Les statuts restent des observations Forgejo non vérifiées et n’autorisent ni verdict ni fusion.

Cette tranche ne démontre pas le parcours LLM ni un gain comparatif. Le lot global reste incomplet. La prochaine action utile reste l’audit dans le scratch Forgejo des préconditions obligatoires pour tous les writers et d’une publication LFS mise en attente puis promue de façon récupérable; aucun patch serveur n’est adopté par ce travail.
