# FORGEJO-PRIVATE-404-RECONCILE-003

## Résultat observé

Dans une instance Forgejo locale jetable, le propriétaire a retiré au collaborateur non-admin son accès read-only à un dépôt privé après une réconciliation positive. Une seconde réconciliation de deux livraisons natives distinctes a fait exactement un `GET /api/v1/repos/forge-admin/agent-forge-core/issues/1`, observé par le proxy loopback : HTTP 404, avec correspondance en mémoire du token lecteur. Le CLI est sorti avec le code attendu `1` et a retourné `inconclusive`, `resource_not_found`, `retryable=false` pour chacune des deux livraisons. Le message explique que le 404 ne distingue pas une issue absente d'un dépôt inaccessible. Une lecture séparée par le propriétaire confirme que l'issue existe toujours. Le journal append-only reste identique avant et après.

Le token est fourni au reconciler par pipe privé, consommé, jamais écrit dans les reçus. L'observateur ne conserve que la méthode, la route, le statut et un booléen de correspondance du token. Le hook est désactivé par readback; Forgejo, receiver et proxys s'arrêtent, et les ports 3130/3131 ainsi que les ports temporaires suivis n'ont plus de listener. Les snapshots sont dans [private-404-reconciliation.json](private-404-reconciliation.json), [api-observation-proxy.json](api-observation-proxy.json), [result.json](result.json) et [cleanup-readback.json](cleanup-readback.json).

## Reproduction et versions

Commande exacte : [command.txt](command.txt). L'état local privé est conservé hors des preuves dans `/tmp/forgejo-private-404-reconcile-003-state`. Forgejo `17.0.0+gitea-1.22.0`, binaire SHA-256 `e5ec437cd6357e8d1b4580cb47ab1daa58a32b33971a63db63446a535d33adb7`; commit de build non établi. Le code source Forgejo audité ailleurs reste au commit `0d791421f85ca13a63b665053d81ea765fbbc571`; cette preuve n'associe pas le binaire à ce commit. Python 3.12.3, Cargo/Rust 1.95.0, Bubblewrap 0.9.0; versions et build isolé détaillés dans [toolchain.txt](toolchain.txt), [provenance.json](provenance.json) et [build-provenance.json](build-provenance.json). Build local des binaires Rust avec `--offline --locked` sous Bubblewrap, réseau et namespaces PID isolés, sources et sysroot en lecture seule.

## Vérifications

- Tests du driver de replay : 20/20; ils acceptent le 404 contrôlé, mais refusent succès mensonger, erreur manquante, lignes réordonnées, retry autorisé, requête supplémentaire, mauvaise route/statut ou credential différent. Sortie : [focused-tests.log](focused-tests.log).
- Suite Python du dépôt : 184/184, dans [python-tests.log](python-tests.log).
- Garde licence Cargo : 7/7; `cargo metadata --locked --offline` confirme `license=null`, `license_file=null`, `publish=[]`, `no_license_selected`, droits `not_assessed`. Sorties : [cargo-license-tests.log](cargo-license-tests.log), [cargo-license-verify.json](cargo-license-verify.json) et [cargo-metadata-readback.json](cargo-metadata-readback.json). Aucune licence n'est choisie.
- `cargo fmt --all -- --check`, `py_compile` des fichiers modifiés et `git diff --check` passent. Le test Rust complet n'a pas été relancé dans cette tranche.
- La première tentative du runner s'est arrêtée avant la révocation, sur un import `quote` manquant; la seconde sur une constante lecteur mal qualifiée. Ces erreurs de harness sont conservées dans les répertoires `forgejo-private-404-reconcile-001` et `forgejo-private-404-reconcile-002`; elles n'ont pas été présentées comme des succès. Le run 003 est reparti avec un nouvel état et un nouveau dossier de preuves. Journal final : [e2e-output.log](e2e-output.log).

## Portée et limites

C'est une seule issue, deux livraisons et une instance locale sous le même UID. Le contrôle d'accès révélé par Forgejo est un 404 indistinguable, à lui seul, d'une ressource absente; le readback propriétaire fournit le témoin positif dans ce fixture. Cela ne mesure ni charge concurrente, ni reprise après interruption, ni comportement d'un LLM/AgentOS, ni comparaison avec CI conventionnelle. Le proxy est un instrument de test, pas une frontière de confiance de production. Aucun service externe, coût, secret réel, publication ou code Forgejo/GitSpace n'est impliqué. Ce lot ne démontre pas un gain général et le projet reste incomplet.

Manifeste de ce dossier : [SHA256SUMS](SHA256SUMS).
