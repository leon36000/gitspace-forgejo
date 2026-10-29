# GITLAB-PC4-SECURITY-REVIEW-001 — état distant et rapport Gitleaks non masqué

Date : 28 octobre 2026. Cible : projet privé GitLab PC4 `root/gitspace-forgejo`, ID 4. Vérification en lecture seule via le connecteur GitLab PC4 déjà authentifié.

## État relu

- Branche par défaut `codex/pc6-transfer-2026-10-28`, commit `6c6ffa3d7d37d48b9d7877acb982d73a2c49a14f`, protégée, sans droit de push pour ce compte.
- Branche de travail `codex/pc6-transfer-work-2026-10-28`, commit `9ac877767e9b458a9cd911e67a10f87aeeaefbdb`, non protégée et inscriptible pour ce compte.
- Aucun merge request ouvert. Le pipeline 328 sur le dernier commit échoue; Semgrep et Trivy réussissent, Gitleaks échoue et le job de fallback est ignoré.

## Résultat de sécurité

Le job Gitleaks 627 a scanné 221 commits, environ 105,27 MB, et signalé 988 occurrences de la règle `generic-api-key`. L’artefact JSON contient 988 champs `Secret` non vides, répartis entre 336 chemins et 37 commits; les valeurs n’ont pas été copiées dans les fichiers locaux ni imprimées dans ce reçu. Leur authenticité n’a pas été vérifiée, donc aucune déduction « faux positif » n’est faite. Cela contredit le reçu TRANSFER-012 qui indiquait les champs `Secret` et `Match` masqués : cette relecture confirme seulement que `Secret` est rempli; elle n’inspecte pas le contenu des valeurs et ne conclut pas sur le masquage de `Match`.

La trace du job contient aussi un jeton d’upload d’artefact non masqué dans une ligne de transfert. La réponse de l’outil a livré cette trace brute, et le premier filtre de sortie a laissé le jeton apparaître dans une sortie d’outil. Il n’a pas été réutilisé ni écrit dans un fichier local; traiter ce jeton comme exposé et vérifier/renouveler son autorisation côté runner s’il reste actif. Sa valeur n’est pas conservée ici. L’artefact du rapport était encore disponible lors de la lecture; sa date d’expiration annoncée est le 11 novembre 2026.

## Décision et limites

Aucun fichier, branche, artefact, réglage ou pipeline distant n’a été modifié par cette inspection. Aucun courriel n’a été ouvert : le connecteur GitLab était déjà suffisant pour cette lecture et un ancien reçu documente qu’un PAT de courriel visait d’autres projets et avait été exposé. Ne pas réutiliser ce PAT.

Une erreur locale a d’abord remplacé `local-run.md` et `commands.txt` dans le dossier historique TRANSFER-004. Les deux fichiers ont été restaurés depuis la branche GitLab et correspondent aux empreintes historiques `2cc10ef44b6778762be4e658e557b7e51528ea697f88acca4e0bc24189ecc638` et `684505452e0e5a2ec29053d045e19e582f115ddc09d11c2b7d3ad2870caaa68f`. Ce nouvel audit reste ici dans son propre dossier. Le seul nouveau fichier de contenu ajouté au dossier historique est `remote-readback.json`; son empreinte a été ajoutée au manifeste local sans réécrire son préfixe, et le vérificateur confirme six fichiers sur six.

Il faut classifier les findings et protéger l’artefact/la trace avant toute nouvelle écriture sur PC4. Les clés réelles doivent être renouvelées par leur propriétaire; le job doit ensuite être configuré pour ne pas exposer les valeurs dans ses artefacts/logs. Le travail de ce checkout reste exécuté sur PC6 : l’API GitLab ne donne pas un terminal sur PC4. Le transfert complet et la validation PC4 restent non démontrés.

Les métadonnées et compteurs sans contenu sensible sont consignés dans `remote-readback.json`; l’empreinte du rapport local et les commandes sont listées dans `commands.txt`.
