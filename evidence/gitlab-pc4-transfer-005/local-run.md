# GITLAB-PC4-TRANSFER-005 — readback documentaire et pipeline — 28 octobre 2026

## Résultat vérifié

Le commit documentaire `20af77b68dcf6dd2cd8282a0011ec7f4b58ba9bb` a été créé sur le projet privé `root/gitspace-forgejo` (ID 4), branche `codex/pc6-transfer-work-2026-10-28`. Il ajoute les six fichiers du reçu TRANSFER-004 et actualise `docs/checkpoint.md` et `evidence/SHA256SUMS`, soit 8 chemins et 3 803 087 octets envoyés en texte UTF-8.

Lecture après commit : les 8 Git blob IDs et modes renvoyés par les arbres GitLab sont identiques aux empreintes Git locales attendues (8/8; aucun absent ni divergent). Le diff GitLab indique 749 ajouts et une suppression sur 8 chemins. Le commit est la tête de la branche de travail lors de la lecture.

Le pipeline 173, lié au SHA `20af77b68dcf6dd2cd8282a0011ec7f4b58ba9bb`, est **success**. Les jobs `pc4_gitleaks` (614), `pc4_semgrep` (615), `pc4_trivy` (616) et `pc4_ci_fallback` (617) sont tous en succès. Le pipeline précédent 172 du transfert source CONT-436 reste lié au SHA 984a9385… et était également success.

La tête lue de la branche par défaut protégée `codex/pc6-transfer-2026-10-28` est `6c6ffa3d7d37d48b9d7877acb982d73a2c49a14f`; cette opération n’y écrit pas. Aucun MR, merge, force-push, email, PAT ou secret n’a été utilisé. Les sources et tests s’exécutent toujours sur PC6; l’accès GitLab est par connecteur API, sans shell PC4.

## Portée du transfert et limite restante

Ce reçu complète la synchronisation des 80 chemins du snapshot CONT-436 (5 205 980 octets, blob/mode 80/80 au SHA 984a9385…) et de ces huit fichiers de suivi. Il ne signifie pas que chaque fichier local du worktree a été transféré. L’inventaire local de ce contrôle compte 15 425 entrées de statut Git; après exclusion des seuls répertoires d’état `.git`, `.runtime`, `.claude`, `target`, `.venv` et caches, 15 294 fichiers réguliers totalisent 199 859 219 octets. Ce chiffre n’est pas un delta distant: les chemins et contenus n’ont pas tous été comparés, et l’inventaire n’a pas encore classé les binaires, bundles, snapshots tiers ou traces susceptibles de contenir des données sensibles. Ne pas effectuer un envoi massif avant réconciliation des chemins et audit de chaque catégorie.

Prochaine étape: comparer les chemins/blobs locaux modifiés aux refs de travail distantes, classer les restes du dépôt, exclure explicitement données privées, secrets, état d’exécution et copies de code tiers non autorisées, puis transférer par lots utiles vérifiés. Le projet produit reste incomplet selon le checkpoint; aucun gain comparatif général n’est établi.
