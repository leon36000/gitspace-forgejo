# FORGEJO-REVIEW-PAGE-401-REFUSAL-001 — résultat local

Date d’exécution : 25 octobre 2026. HEAD de base : `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. Le worktree est modifié; le test et l’entrée de runner utilisés ici ne sont pas contenus dans ce commit. Le runner a imprimé l’empreinte de l’arbre de sources Cargo copié dans la sandbox : `1604f27412a974502310063cc388bf8af1358a96a09b1231b4a800836870e906`.

## Question vérifiée

Une lecture paginée partielle des avis peut-elle autoriser ou poursuivre un merge lorsque Forgejo refuse l’accès à la page suivante ?

## Résultat observé

Le test processus ignoré par défaut `unauthorized_second_review_page_stops_merge_before_authority` traverse le binaire `forgejo-reconcile-local` et une fixture HTTP loopback. La première page annonce 51 entrées et contient 50 avis approuvés; la seconde page renvoie HTTP 401 avec un marqueur synthétique. Le processus sort avec un objet JSON `inconclusive`, `merge_authorized=false`, `merge_attempted=false`, `error.stage=review-list-read`, `error.code=upstream_unauthorized` et `error.retryable=false`.

La fixture confirme les lectures des deux pages, aucun POST et aucune lecture `branch_protections`; elle n’est pas marquée fusionnée. Le marqueur d’erreur et le token synthétiques sont absents de stdout, et `output_json` exige un stderr vide. Le runner sous Bubblewrap passe 1/1, 0 échec, 0 ignoré et 5 tests filtrés. Il emploie `--unshare-net --unshare-pid`; Cargo compile avec `--offline --locked`.

Le chemin runtime observé est `forgejo_review_pages` dans `src/bin/forgejo-reconcile-local.rs` (environ ligne 2973), qui transforme le statut 401 en `ApiReadError::Unauthorized`; `gate_api_failure_code` le mappe en `upstream_unauthorized` non réessayable (environ ligne 2888). Le lecteur d’autorité retourne ce refus à l’étape `review-list-read` avant la validation de la protection de branche (appel autour de la ligne 4297). Ce lot n’a modifié que le test processus et son runner, pas le runtime.

## Périmètre et limites

La fixture vérifie un refus d’autorisation après une première page positive et l’absence de décision/effet de fusion en aval. Elle ne teste pas un serveur Forgejo vivant, les permissions réelles du compte, des revues concurrentes, ni le comportement après fusion. Le test caractérise le runtime actuel; il ne démontre pas un parcours LLM, une amélioration comparative ou un gain de productivité. Aucun secret réel, service externe, agent, modèle ou coût modèle n’est utilisé.

## Fichiers de preuve

- `commands.txt` : commande ciblée.
- `runner.log` : sortie complète du runner et du test isolé.
- `test-result.txt` : résumé du résultat et statut de sortie.
- `tool-versions.txt` : versions et plate-forme.
- `source-sha256.txt` : empreintes des entrées et sources consultées.
- `SHA256SUMS` : intégrité des fichiers de ce rapport.
