# GITLAB-PC4-TRANSFER-006 — inventaire des preuves restant à synchroniser — 28 octobre 2026

## Méthode et résultat local

Comparaison des chemins modifiés/non suivis du worktree PC6 avec l’arbre récursif GitLab figé au commit `796a6085cb71cd7d2ae4c3be7c53c3d401691448` (482 entrées, 446 blobs). Le vérificateur `audit_export.py` conserve l’inventaire distant et produit un manifeste local par chemin, Git blob SHA-1, mode et taille. Il exclut son propre répertoire de reçu ainsi que `docs/checkpoint.md` et `evidence/SHA256SUMS` (mis à jour par l’audit) pour éviter une boucle de manifeste; ces deux fichiers seront synchronisés séparément avec le reçu final.

Au snapshot: 15 438 entrées `git status`; 15 297 fichiers réguliers après exclusion des deux fichiers de suivi; 15 085 fichiers texte UTF-8 ont passé les filtres. Le rapprochement trouve 370 blobs identiques et 14 715 chemins absents (aucune divergence dans la liste), tous sous `evidence/`, pour 58 717 295 octets. Le balayage n’a trouvé aucune correspondance aux six motifs connus (GitLab/GitHub token, AWS access key, clé privée PEM, long Bearer, assignation de credential). Il exclut aussi .git/.runtime/.claude/target/venv/caches, formats binaires/archives et .gitconfig, fichiers NUL/non UTF-8, ainsi que 108 snapshots source Go/Rust/C/H/proto sous evidence.

Les 14 715 fichiers sont des candidats textuels, pas une garantie qu’ils soient tous dépourvus de données privées ou d’extraits tiers; les filtres sont heuristiques et non exhaustifs. La liste exacte, avec Git blob SHA-1 et mode, est dans `candidate-manifest.jsonl`. Ce lot est un inventaire avant import, pas une affirmation que le transfert soit complet.

## Suite du transfert demandé

Transférer les preuves par lots sur la seule branche privée `codex/pc6-transfer-work-2026-10-28`, relire les blobs et modes, puis exécuter une CI complète sur le SHA final. Les lots intermédiaires peuvent utiliser `[skip ci]` pour éviter les scanners à chaque lot; GitLab documente ce comportement pour les pushes, sauf politique contraire ([documentation GitLab sur les pipelines](https://docs.gitlab.com/ci/pipelines/)). Le dernier lot sans marqueur déclenchera la CI. Aucune modification de la branche par défaut, MR ou fusion.

Limites: inventaire local basé sur `git status`, pas une migration de l’historique Git. Les tests restent sur PC6; les écritures se font via API GitLab, sans shell PC4.
