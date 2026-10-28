# FORGEJO-MERGE-MISSION-PR-BINDING-001 — liaison admission, PR et credential

## Résultat

Le 26 octobre 2026, deux tests `cfg(test)` supplémentaires exercent la logique réelle du gate local à travers son client HTTP loopback.

Le test de refus prépare une admission MISSION-012 valide pour l’issue 1, un proof signé pour le profil/dépôt/base correspondants et un candidat présenté sur la PR 23. Il modifie ensuite le fichier hôte `HostReviewBaseBindingV1` pour lier la revue à la PR 24. Le gate retourne `rejected`, `merge_attempt_binding_mismatch`, `retryable=false`, avec une action demandant de restaurer les mêmes entrées hôte ou d’admettre une nouvelle génération; le détail conserve la PR 23 de la liaison de tentative. `merge_attempted=false` et le serveur loopback ne reçoit **aucune requête**, donc aucune lecture Forgejo ni POST de fusion ne précède le refus.

Le test positif utilise le profil MISSION-012 avec des credentials fixture distincts pour l’acteur de merge et le lecteur de politique. Il vérifie la séquence HTTP attendue dans le seul dépôt compilé, le choix du credential de lecteur sur les routes de protections, le credential de merge sur les autres routes, exactement un POST vers `/pulls/23/merge`, et le résultat attaché au profil, au dépôt, à la branche et au SHA de fusion fixture. Aucun token ne figure dans le résultat sérialisé. Le harness enregistre route et credential, pas le corps du POST; la validation du corps strict est couverte séparément par les tests existants du client.

## Vérifications reproductibles

- Refus ciblé : `mission-final.log`, 1/1.
- Liaison positive et credentials séparés : `mission-positive-final.log`, 1/1.
- Tous les tests unitaires du binaire modifié : `bin-suite-128.log`, 128/128.
- `cargo fmt --all -- --check`, `git diff --check`, garde Cargo de licence et ses tests : sorties `*-final.log`; garde `verified`, tests 9/9.
- Commandes, versions et hashes du source courant : `commands.txt`, `tool-versions.txt`, `source-sha256.txt`.

Les premiers essais sont conservés comme traces de développement, pas comme verdicts finaux : `focused.log` avait une assertion du harness trop restrictive pour la lecture d’identité `/users/verifier-bot`; après correction, le probe générique passe dans `focused-final.log`. Dans le premier test MISSION-012, l’assertion attendait le refus plus tardif `invalid_review_base_binding`; le résultat observé, `merge_attempt_binding_mismatch` sans aucune requête HTTP, montre que la cohérence de la tentative durable refuse plus tôt. L’assertion a été alignée sur ce contrat et le run final passe.

## Portée et limites

Le code de production n’a pas changé dans ce lot : les ajouts sont exclusivement des fonctions de test dans `src/bin/forgejo-reconcile-local.rs`. Il s’agit d’un serveur HTTP loopback synthétique sous le test Rust, pas d’une instance Forgejo. Le test prouve le refus de la mauvaise liaison PR avant réseau et la sélection des deux credentials pendant un parcours positif fixture; il ne prouve pas leurs permissions côté Forgejo, l’exclusion des writers externes, une comparaison atomique de base ou un merge live. Le POST fixture n’est pas exécuté sur un Forgejo réel. Aucun LLM, AgentOS, RAG, benchmark apparié, coût fournisseur ou service externe n’est utilisé.

État Cargo revérifié : `license=null`, `license_file=null`, `publish=[]`, `no_license_selected`, droits de redistribution `not_assessed`, marqueur SPDX `NOASSERTION`. Aucun choix de licence n’est fait. Le projet reste incomplet : la comparaison contrôlée et l’intégration à un runtime LLM/AgentOS effectivement autorisé ne sont pas démontrées; l’autorité serveur contre tous les writers Forgejo demeure hors de portée de ce test.
