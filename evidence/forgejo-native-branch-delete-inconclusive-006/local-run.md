# FORGEJO-NATIVE-BRANCH-DELETE-404-INCONCLUSIVE-003

## Parcours observé

Une instance Forgejo locale et jetable `17.0.0+gitea-1.22.0` a été démarrée sur loopback. Un dépôt privé a été créé avec un lecteur non-admin `read` (push=false, pull=true). Après inscription d’un hook `delete` dirigé uniquement vers le receiver loopback, Git HTTP a créé puis supprimé `agent/native-delete-proof`. `git ls-remote --heads` confirme que la ref a disparu. Le hook natif a remis un événement `delete` dont `ref` vaut le nom court `agent/native-delete-proof`, `ref_type=branch`; le task Forgejo portant le même UUID est livré avec succès, HTTP 202.

Le vrai receiver et `forgejo-reconcile-local` ont été reconstruits hors ligne sous Bubblewrap. Le CLI a relu l’API Forgejo avec le credential du lecteur via FD privé et une seule lecture upstream. Sa sortie, code 1, est `schema_version=4`, `verdict=inconclusive`, `error.code=resource_not_found`, `retryable=false`, sans `canonical_state`. Son action explique qu’un 404 ne permet pas de distinguer branche absente et ressource inaccessible. Le journal d’inbox reste octet pour octet identique; le résultat ne devient pas un faux succès. Le reçu de cette run contient l’événement, le task, l’erreur structurée, les hashes du binaire et les provenance sources.

L’instance et le receiver sont arrêtés avec code 0, le hook est désactivé et relu, les deux listeners sont absents. Aucun code Forgejo n’a été copié ou modifié.

## Tentatives du harnais

Les tentatives 003–005 sont préservées avec leurs manifestes. 003 échouait pendant une lecture API de diagnostic redondante avant l’appel du reconciler; 004 avait une étape trop large; 005 a capturé la vraie sortie 404, mais le validateur du harnais exigeait à tort un champ `resource` au sommet. Le schéma courant place cette ressource sous `error.details.resource`. Le validateur et ses tests ont été corrigés; la run 006 passe. Les trois premiers runs ont confirmé le nettoyage des services/hooks et ne sont pas présentés comme réussis.

## Vérifications et contexte

- Tests Python du driver : 57 réussis; les trois tests du validateur couvrent refus inconclusif attendu, faux succès refusé et cas positif après recréation.
- `py_compile`, `cargo fmt --all -- --check` et `git diff --check` passent. La suite Rust globale courante (run précédent, sans changement de code Rust dans ce lot) passe 519 tests, 0 échec, 63 ignorés sur 34 groupes.
- Cargo readback frais : `license=null`, `license_file=null`, `publish=[]`; `[package.metadata.agent-forge.licensing].decision=no_license_selected`, `redistribution_rights=not_assessed`, et `publish=false`. Aucune licence n’est attribuée et aucune permission de redistribution n’est inférée.
- Recherche de source locale : `AgentRuntimeV1` est déclaré dans `src/agent_runtime.rs`; l’unique implémentation trouvée est `FixtureRuntime` dans les tests. Aucun connecteur fournisseur de modèle n’est trouvé dans `Cargo.toml`, `src` ou `tests`. Aucun secret/env n’a été inspecté; aucun modèle/service externe n’a été appelé.

Forgejo source audité: tag `v17.0.0`, commit `0d791421f85ca13a63b665053d81ea765fbbc571`; le binaire local a SHA-256 `e5ec437cd6357e8d1b4580cb47ab1daa58a32b33971a63db63446a535d33adb7`, mais son commit de build n’est pas établi. Le code source officiel est cité dans [le relevé du wire delete](../forgejo-native-branch-delete-reconcile-002/forgejo-source-reference.txt).

## Limites

Un run sur une seule instance/dépôt/branche ne prouve pas le comportement multi-tenant, la charge, un modèle LLM ou AgentOS, une PR/fusion, ni un avantage comparatif. Le token est une identité locale de test limitée en lecture; rien n’est publié ou déployé. La réponse reste volontairement inconclusive sur 404.
