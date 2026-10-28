# FORGEJO-PRIVATE-404-RECONCILE-004 — correction d'accès et reprise vérifiée

## Résultat observé

Dans une instance Forgejo locale jetable, deux replays natifs du même webhook conservent des UUID distincts et sont réconciliés vers une issue privée. Le lecteur non-admin commence avec le droit `read`; l'observateur voit un seul GET HTTP 200 pour les deux lignes. Le propriétaire retire ce droit, puis le même lot est relancé : l'observateur voit un seul GET HTTP 404 authentifié avec le token du lecteur. Le CLI sort avec le code 1 et donne deux lignes `inconclusive/resource_not_found`, `retryable=false`, sans état canonique. L'action retournée précise que le 404 ne distingue pas une issue absente d'un dépôt inaccessible. Le propriétaire confirme que l'issue existe toujours.

Le propriétaire rétablit ensuite le droit read-only et le même lot, avec les mêmes UUID, est relancé dans un nouveau processus. Un seul GET HTTP 200 est observé; le CLI sort avec le code 0 et retourne les deux états courants `open`. Les octets du journal receiver restent identiques avant/après les trois réconciliations. Les credentials sont passés par des pipes privés, consommés par chaque invocation et jamais écrits. Le proxy garde uniquement la méthode, le chemin, le statut et un booléen de correspondance du token. Les résultats sont dans [reconciliation.json](reconciliation.json), [private-404-reconciliation.json](private-404-reconciliation.json), [private-404-recovery.json](private-404-recovery.json) et [api-observation-proxy.json](api-observation-proxy.json).

Le hook est désactivé avec readback. Forgejo, le receiver et les proxys s'arrêtent sans erreur; les ports 3130/3131 et ports temporaires suivis sont fermés. État privé conservé hors du dossier de preuve : `/tmp/forgejo-private-404-reconcile-004-state`; voir [cleanup-readback.json](cleanup-readback.json).

## Contrat d'interface agent constaté

Le point d'extension du produit est `AgentRuntimeV1::execute` dans `src/agent_runtime.rs:4492`; son unique implémentation repérée est `FixtureRuntime` dans `tests/agent_runtime_contract.rs:50`. Le réexamen local de l'arbre PC5/RAG n'a trouvé qu'un candidat non propre, pas un endpoint vivant ni un fournisseur d'exécution de tâches de code. L'API Python observée porte sur les capacités RAG et les sources authentifiées; aucun appel au service ou à un modèle n'a été fait. Les commits, versions, empreintes, limites et décision de ne pas brancher ce RAG comme orchestrateur sont détaillés dans [agent-interface-reassessment.md](agent-interface-reassessment.md). Aucun agent n'a exécuté cette tranche; l'intégration LLM reste non démontrée.

## Commandes, versions et vérifications

La commande Forgejo exacte est dans [command.txt](command.txt). Le binaire `17.0.0+gitea-1.22.0` a SHA-256 `e5ec437cd6357e8d1b4580cb47ab1daa58a32b33971a63db63446a535d33adb7`, mais son commit de build n'est pas établi comme étant le commit source audité `0d791421f85ca13a63b665053d81ea765fbbc571`. Versions, politiques de build et hashes des binaires Rust sont dans [toolchain.txt](toolchain.txt), [provenance.json](provenance.json) et [build-provenance.json](build-provenance.json). Les binaires Rust ont été reconstruits avec `--offline --locked` sous Bubblewrap, namespaces réseau/PID isolés et sources/sysroot en lecture seule.

- Tests ciblés du driver : 20/20; suite Python complète : 184/184, sorties dans [focused-tests.log](focused-tests.log) et [python-tests.log](python-tests.log).
- Le garde Cargo passe 7/7. `cargo metadata --locked --offline` confirme `license=null`, `license_file=null`, `publish=[]`, décision `no_license_selected`, droits `not_assessed`. Aucune licence n'a été choisie; publication Cargo reste désactivée. Readbacks : [cargo-license-verify.json](cargo-license-verify.json), [cargo-license-tests.log](cargo-license-tests.log), [cargo-metadata-readback.json](cargo-metadata-readback.json).
- `cargo fmt --all -- --check`, `py_compile` des deux fichiers Python modifiés et `git diff --check` passent; sortie E2E complète : [e2e-output.log](e2e-output.log).

## Portée et limites

Une issue et deux livraisons dans une seule instance sous le même UID; une seule répétition du cycle d'accès. Le test prouve que l'erreur négative ne persiste pas entre invocations après correction de permission et que le même lot peut être repris. Il ne prouve pas une reprise après crash entre écriture et merge, la concurrence sous charge, un AgentOS ou un LLM, ni un gain comparatif. Le proxy loopback est un instrument de test, pas une frontière de sécurité de production. Le 404 seul demeure ambigu; l'API propriétaire et la séquence d'autorisation du fixture lèvent cette ambiguïté dans cette expérience. Aucun service externe, secret réel, coût, publication ou code Forgejo/RAG n'a été utilisé ou copié.

Le manifeste local passe 27/27. Trente-deux nouveaux hashes ont été ajoutés au journal global append-only; les 33 chemins de preuve/source correspondants ont zéro divergence à leur dernière occurrence. L'historique global (six lignes anciennes mal formées) n'est pas réécrit. Voir [SHA256SUMS](SHA256SUMS). Le projet reste incomplet.
