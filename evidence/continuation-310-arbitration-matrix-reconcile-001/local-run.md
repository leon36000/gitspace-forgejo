# CONTINUATION-310 — correction de la ligne d’arbitrage dans la matrice

Date : 27 octobre 2026. Worktree existant fortement modifié, HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. Le lot ne change aucun code exécutable, n’exécute aucun serveur, ne lance aucun modèle et ne modifie pas la licence Cargo.

## Question

La ligne 22 de `docs/refusal-evidence-matrix.md` disait encore que `Requirements` ne conservait ni PR source ni décisions et qu’aucun test ne validait la liaison des revues. Cette conclusion correspond aux audits pré-implémentation CONTINUATION-296/297; elle contredisait les sources et reçus ultérieurs CONTINUATION-299/301.

## Vérification des sources

Lecture de `Requirements`, `RequirementArbitrationDecisionV1`, `MissionArbitrationSourceV1`, `evaluate_requirement_arbitration`, `validate_requirement_document_v3` et `LocalLoopbackForgejoSource`. Les reçus CONT-299 et CONT-301 référencent les mêmes empreintes courantes pour les sources exécutables pertinentes : `src/lib.rs`, `src/mission_admission.rs`, `src/bin/forgejo-mission-local.rs`, ainsi que les tests CLI/contrat correspondants. Les empreintes de `docs/checkpoint.md` et `docs/refusal-evidence-matrix.md` ont évolué depuis les snapshots de ces reçus et ne sont pas présentées comme inchangées. Les résultats de comparaison source par source sont conservés dans `source-hash-reconcile.log`.

L’implémentation v3 exige un numéro de PR source et des décisions structurées; compare le contenu d’exigences protégé avec le contenu de cette PR à son commit de fusion; vérifie le hash et le scope exacts de la policy hôte; charge les pages complètes de revues; vérifie la PR et les avis avant/après; et ne retient que les approbations officielles, actuelles, non périmées/non rejetées sur le head exact, d’IDs autorisés selon la policy. Les ambiguïtés ouvertes, policy/trace manquante, octets divergents, pages incomplètes ou avis insuffisants refusent l’admission avant allocation.

Les preuves réutilisées sont exactement les portées rapportées : CONT-299, contrats d’arbitrage v3 **12/12** et tests ciblés du CLI; CONT-301, processus du vrai binaire CLI contre une API HTTP synthétique **1/1**, avec un succès fixture et les refus policy absente / reviewer non autorisé sans modification du journal. Aucun test n’a été rejoué pour ce lot de documentation.

## Correction et limites

La matrice dit maintenant que le chemin v3 existe et distingue ses contrôles exécutés des preuves qui restent ouvertes. L’ID de compte `9001` de la fixture ne prouve pas une personne et ne permet pas d’exclure un bot; Forgejo v17 ne publie pas `IsBot` dans son DTO public. Aucun compte, PR, avis ou policy réel n’est présent. Les lectures paginées et leur double comparaison ne forment pas un snapshot atomique et ne suppriment pas la fenêtre après la dernière lecture. L’exécution d’agent reste `false` dans le reçu CONT-301.

Commandes, versions disponibles et empreintes de ce rapprochement : fichiers adjacents. `git diff --check` est exécuté après l’édition. Cette tranche corrige un document obsolète; elle ne démontre pas une revue humaine, une intégration Forgejo v3 live, une livraison agent ou un gain comparatif.
