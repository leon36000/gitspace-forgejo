# BENCHMARK-PROTOCOL-PREPARATION-001

Date : 26 octobre 2026. Revue documentaire et rédaction locale dans le worktree du produit; HEAD examiné `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`, dépôt fortement modifié. Aucun commit, LLM, service externe, dépense, publication ni accès de production.

## Résultat

La section [Plan de mesure](../../docs/architecture.md#plan-de-mesure) formalise un protocole apparié Forgejo + agent + CI conventionnels contre le même socle avec la couche agentique. Elle fixe les contrôles d’appariement (commit, demande, modèle/configuration, budget), les sessions isolées et leur ordre randomisé, un pilote de calibration de 12 tâches × 3 répétitions × 2 bras, une règle distincte pour l’effectif confirmatoire, l’oracle protégé exécuté indépendamment, les définitions des critères satisfaits/régressions/interventions/délai/tokens/coût, l’analyse des variations, la conservation des échecs et les règles de reprise. L’ablation reste confinée aux copies isolées et ne retire pas de garde de sécurité d’une instance partagée.

La référence scientifique de format est le papier [SWE-bench (arXiv:2310.06770)](https://arxiv.org/abs/2310.06770), pas son dataset ou ses scores. La page d’analyse [OpenAI sur la validité de SWE-bench Verified](https://openai.com/index/why-we-no-longer-evaluate-swe-bench-verified/) a été consultée le 26 octobre 2026; elle rapporte des problèmes de conception de tests et d’exposition des solutions sur des tâches publiques. Ce constat motive ici les tests protégés audités, le choix de tâches récentes et la consignation de l’incertitude de contamination.

Ce travail écrit la méthode seulement. `protocol-status.json` confirme `execution_status=not_run`, zéro tâche sélectionnée et zéro appel de modèle. Le nombre 72 est la taille prévue du pilote de calibration, pas un échantillon observé ou une puissance confirmatoire. Aucun coût ni nombre de tokens n’est attribué à ce lot; l’absence de relevé n’est pas traitée comme zéro coût. Le mécanisme de plafond Claude précédemment utilisé n’a pas démontré une limite de facturation fiable et n’autorise aucun nouvel appel.

Pendant la revue finale, le routage issue/PR de CONTINUATION-071 a été recoupé avec la forme source Forgejo v17.0.0; le risque de mauvais discriminateur n’est pas reproduit. La preuve de code et ses hashes figurent dans [`forgejo-issue-comment-routing-review.md`](forgejo-issue-comment-routing-review.md). Une recherche dans `src`, `scripts` et `tests` ne trouve aucun consommateur du corps dans un contexte de mission interne; la sortie CLI reste un contrat externe hostile et consultatif, pas un flux LLM exercé. Le refus précoce d’admission depuis un commentaire est protégé séparément par [ISSUE-COMMENT-ADMISSION-REFUSAL-001](../issue-comment-admission-refusal-001/local-run.md).

## Préconditions non satisfaites

- Corpus de tâches réelles gelé, droits et licences de chaque dépôt/tâche documentés, critères et oracle indépendants protégés.
- Runtime/agent réellement autorisé et stable, version/configuration reproductible et métadonnées d’usage disponibles.
- Limite de coût fournisseur qui arrête les appels de façon vérifiable, avec autorisation numérique explicite.
- Pilote exécuté puis effectif confirmatoire calculé avant ouverture des résultats et avant toute revendication de gain.

Il n’y a eu aucun test logiciel à relancer pour cette édition documentaire. Les références de sources, outils, commandes de cohérence et manifestes append-only sont conservées ici. La mission reste incomplète.
