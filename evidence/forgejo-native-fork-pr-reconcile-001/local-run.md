# FORGEJO-NATIVE-FORK-PR-RECONCILE-001

## Résultat

Sur une instance Forgejo locale jetable, le runner a créé le dépôt cible privé forge-admin/agent-forge-core, le compte contributeur forge-review-author et son fork privé forge-review-author/agent-forge-core dont le parent est le dépôt cible. Le compte contributeur a créé une branche, un fichier candidat relu par l’API, puis une PR Forgejo native #2 vers main.

Le readback de la PR à l’ouverture a confirmé le dépôt base forge-admin/agent-forge-core, le dépôt head forge-review-author/agent-forge-core, le SHA base e6ff856f778f026adb9b537afc0bd4e5e0b7a50a et le SHA head d6cd2f8b973a1d02618f457dcee11072d7d9216e. Le webhook natif pull_request/opened porte ces mêmes dépôts et SHA; le parseur du runner vérifie les noms base/head attendus. La suite unitaire confirme qu’un dépôt base ou head étranger est rejeté.

Forgejo a livré sept tâches webhook; le readback de forgejo-hook-tasks.json indique pour chacune is_delivered=true, is_succeed=true, response_status=202. La réconciliation de l’événement d’ouverture effectue un GET canonique et voit l’état courant après la fermeture de la PR (closed, non fusionnée); ce readback tardif ne prétend pas représenter l’état à l’instant d’ouverture. La PR a été fermée pour terminer le scénario, sans fusion. Le hook a été désactivé par readback et les processus/adresses d’écoute attendus ont disparu.

## Provenance et reproduction

- HEAD produit au lancement : f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0. Le worktree reste fortement modifié; le code de ce lot n’est pas commité.
- Source Forgejo officielle examinée séparément : tag v17.0.0, commit 0d791421f85ca13a63b665053d81ea765fbbc571; ce commit est l’identité du tag source vérifié, pas une provenance du binaire lancé.
- Le binaire local rapporte forgejo version 17.0.0+gitea-1.22.0 (release name 17.0.0) built with GNU Make 4.4.1, go1.26.9 : bindata, timetzdata, sqlite, sqlite_unlock_notify; SHA-256 e5ec437cd6357e8d1b4580cb47ab1daa58a32b33971a63db63446a535d33adb7. Son commit de build exact reste inconnu.
- Hash du runner : 2d7529de9a78911057d4e89db0ba67297ba4cf28adddc943c231e9ceff7c516d. Hash du test : 41f19a582a3f41594fc59293b60608be58dad6610b0f2ee5e95cef7e9dab02d9.
- Outils : Python 3.12.3, Git 2.43.0; le build local consigné dans provenance.json utilise Rust/Cargo 1.95.0 et Bubblewrap 0.9.0.
- La sortie JSON capturée dans console.json égale result.json; une vérification de forme n’y a trouvé aucun matériau de credential. Les secrets d’instance et l’état privé restent hors du dossier de preuve.
- La validation de syntaxe des deux scripts de manifeste par py_compile a laissé deux fichiers bytecode dans __pycache__; ils sont conservés comme sortie d’outil, pas traités comme preuve. L’index couvre les fichiers d’évidence au premier niveau et les sources .py; le vérificateur autorise ce cache uniquement s’il ne contient que des fichiers .pyc ordinaires.

Commande du parcours :

    PYTHONDONTWRITEBYTECODE=1 python3 scripts/run_forgejo_live_webhook_reconcile_emitted.py --forgejo-binary /tmp/forgejo-17.0.0 --state-dir /tmp/forgejo-native-fork-pr-20261026-001 --evidence-dir evidence/forgejo-native-fork-pr-reconcile-001 --fork-pull-request

Après le parcours, PYTHONDONTWRITEBYTECODE=1 python3 -m unittest -v tests.test_forgejo_emitted_webhook_e2e passe 112/112. ast.parse passe sur le runner et le module de test; les plages Python modifiées ne contiennent pas d’espace final; git diff --check réussit pour les changements suivis. python3 scripts/verify_cargo_license_state.py rapporte license=null, license_file=null, publish=[], no_license_selected, redistribution_rights=not_assessed et NOASSERTION.

## Portée et limites

C’est un test d’intégration de Forgejo, du receiver/CLI local et du contrat de webhook; le driver est le banc local. Le compte lecteur API a des scopes read:user, read:repository, read:issue et aucun droit push/admin sur la cible. Le contributeur n’a pas de droit push/admin sur le dépôt cible (pull=true, push=false, admin=false), mais son jeton de création contient des scopes write:issue et write:repository pour gérer son fork et sa PR. Cette exécution ne démontre donc pas un jeton globalement limité au seul dépôt ou une politique générale de permissions minimales.

n=1, une instance et un binaire locaux; aucune fusion, exécution du code candidat, CI, LLM, AgentOS, RAG, mémoire, mesure comparative ou gain de productivité. L’émission native et les noms base/head sont observés pour une PR de fork privée dans cette version uniquement. Les API/routes, droits de chaque type d’événement, comportements d’autres versions et résistance à un processus privilégié ne sont pas couverts.
