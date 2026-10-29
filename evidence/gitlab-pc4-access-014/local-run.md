# GITLAB-PC4-ACCESS-014 — prévol distant en lecture seule

Date : 28 octobre 2026.

## Résultat observé

L’utilisateur a demandé de continuer sur l’instance GitLab PC4. Le connecteur MCP déjà configuré accède au projet privé root/gitspace-forgejo, ID 4. GitLab MCP annonce la version 19.5.1. Aucun courriel, mot de passe, PAT ou autre secret n’a été lu ou utilisé dans ce lot.

La branche par défaut codex/pc6-transfer-2026-10-28 est protégée et pointe sur 6c6ffa3d7d37d48b9d7877acb982d73a2c49a14f. La branche de travail codex/pc6-transfer-work-2026-10-28 est non protégée et pointe sur 9ac877767e9b458a9cd911e67a10f87aeeaefbdb.

Le pipeline 328, sur le SHA de la branche de travail, a l’état failed. Gitleaks 627 a échoué; Semgrep 628 et Trivy 629 ont réussi; pc4_ci_fallback 630 a été skipped. Le détail antérieur du scanner est documenté dans ../gitlab-pc4-transfer-012/local-run.md : 10 chaînes de 64 caractères dans des champs forgejo-read.token et invalid-forgejo-read.token ont une validité/révocation non établie. Aucune valeur n’est recopiée ici.

## État local et limites

Le worktree courant reste local/WEBHOOK-005-canonical-links à HEAD f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0, avec 15 843 entrées de statut modifiées/non suivies au prévol. Le remote pc4-gitlab est configuré. Le transfert antérieur a vérifié exactement 14 715 chemins de son manifeste candidat; cette vérification ne couvre pas automatiquement tous les changements locaux actuels ni l’historique Git complet.

Avant la modification de ce checkpoint et la création de ce reçu, le vérificateur local a validé les dernières empreintes de 14 028 chemins, sans absent/divergent/non haché/dangereux; une seule entrée historique non hachée est signalée. Le reçu ne répète pas le readback des blobs PC4.

Aucune branche, MR, pipeline ou configuration distante n’a été modifié. Les écritures PC4 restent suspendues jusqu’à la décision de l’utilisateur sur les valeurs Gitleaks. La branche actuelle est préservée; aucune révocation, suppression, nettoyage ou réécriture d’historique n’a été faite.
