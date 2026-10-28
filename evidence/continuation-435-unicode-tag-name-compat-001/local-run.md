# CONTINUATION-435 — compatibilité des tags Release avec les espaces Unicode Git

Date : 28 octobre 2026 (America/Montreal).

## Résultat

Le validateur `valid_tag_ref_name` appliquait `str::trim()` et `char::is_whitespace()`. Il refusait donc des noms UTF-8 pourtant acceptés par Git, avant même la réconciliation de l’état Release. Pour `\u{00a0}release/v1.0`, le test rouge retournait `InvalidCurrentStateField("tag_name")`.

Le validateur continue de refuser les contrôles Unicode, l’espace ASCII et les caractères/règles de ref interdits. Il ne rejette plus les espaces Unicode non contrôles. Le reconciler encode toujours le tag par segments avec `encode_url_path_segments`; les trois cas acceptés sont vérifiés de bout en bout dans `FixtureApi` : NBSP initial, espace cadratin dans un segment et séparateur de ligne Unicode après `/`. Le contrôle Unicode U+0085 reste explicitement refusé par la politique locale, même si Git 2.43.0 le laisse passer; cette divergence est volontaire afin d’éviter qu’un contrôle n’entre dans la projection.

## Vérifications

- TDD rouge : test focalisé échoue sur `InvalidCurrentStateField("tag_name")`, code 101.
- Test focalisé corrigé : 1/1.
- `webhook_reconcile_contract` : 103/103.
- `cargo test --offline --locked --all-targets -- --test-threads=1` : 763 réussis, 0 échoué, 200 ignorés, 38 groupes.
- `cargo fmt --all -- --check`, `git diff --check` et le contrôle des espaces finaux : réussite.
- `git check-ref-format` accepte les trois valeurs Unicode du test et refuse les valeurs invalides consignées dans `git-refname-oracle.json`.
- Relecture Cargo hors ligne : `license=null`, `license_file=null`, `publish=[]`, `no_license_selected`, droits `not_assessed`, marqueur personnalisé `NOASSERTION`; les 12 tests du garde passent. Aucun champ de licence ni fichier Cargo n’a été modifié dans ce lot et aucune licence produit n’a été choisie.
- `sha256sum -c` valide les 19 entrées du manifeste de run. Le vérificateur global append-only passe avec 13 597 chemins uniques, aucun absent, divergent, non haché, malformé ou dangereux (1 entrée historique `None`, 6 lignes vides ignorées).

## Portée et limites

Le test couvre la validation locale, la réconciliation Release et l’encodage de l’URL canonique avec des fixtures. Il ne prouve pas qu’une instance Forgejo live accepte ou émet ces caractères, ni le comportement de tous les reverse proxies. Le corpus `git check-ref-format` est ciblé, pas une preuve exhaustive de la grammaire Git. L’implémentation du runtime agent reste une interface avec `FixtureRuntime` dans les tests; aucun LLM/AgentOS, aucune fusion et aucun benchmark n’ont été exécutés. Aucun coût ni service externe n’a été engagé.

Le reçu ne modifie pas l’historique Git local et n’est pas publié. Prochaine étape : confronter le reste des noms de refs projetés par l’adaptateur à Git, en séparant explicitement les contrôles de compatibilité de la politique locale sur caractères de contrôle.
