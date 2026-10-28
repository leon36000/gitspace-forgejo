# SEMANTIC-CLEAN-MERGE-CANARY-001 — canari isolé de conflit sémantique

Date : 26 octobre 2026. Le canari crée un dépôt Git/Cargo temporaire et ne modifie aucun code produit. Il n’utilise ni Forgejo, ni LLM, ni AgentOS. Commande de référence : `python3 evidence/semantic-clean-merge-canary-001/run_canary.py --run-label attempt-004`. Le script reproductible est `run_canary.py`; le résumé est `attempt-004-result-summary.json`.

## Résultat

Deux branches partent du même commit `b325edaf41074c602d7e1f3e0698168c80331dea`. La branche `exclusive-lower` modifie le helper `lower_bound`; `exclusive-upper` modifie un helper distinct, `upper_bound`. Leurs commits sont respectivement `efe1f7f7b4a5fb40740fd461097f2c1ecab96d41` et `49969854e53926a632fb8bcd2caef87493ca0169`. Les tests propres à chaque branche passent individuellement.

`git merge-tree --write-tree --name-only HEAD exclusive-lower` retourne 0, arbre `06174e7f56840a7ab8126ce947d66ab986b3b33d`, sans chemin en conflit. Après fusion, les deux tests de branche restent verts. L’acceptation protégée indépendante demande le comportement défini pour l’intervalle adjacent `[0,1]` lorsque les deux bornes sont exclusives : retourner `Err(EmptyRange)`. Avant correction, le code appelle `i32::clamp(1, 0)` et panique (`min > max`); le test rouge termine avec le code 101. Les deux vérifications superficielles vertes ne révèlent donc pas la panne intégrée.

L’intégrateur a ajouté une vérification `effective_min > effective_max` avant `i32::clamp`, avec retour `Err(BoundsError::EmptyRange)`. L’acceptation protégée passe, puis `cargo test --offline --locked` passe les quatre tests d’intégration (zéro échec). Le snapshot corrigé est `attempt-004-combined-src-lib.rs`. Chaque commande et sortie sont préservées dans les fichiers `attempt-004-*.log`; les empreintes sont dans `attempt-004-source-hashes.json`. Les outils du run sont consignés dans `toolchain.txt`.

## Historique des essais

Les journaux racine `merge-tree.log` conservent le premier montage, qui avait placé les deux changements dans le même hunk; Git l’a refusé comme conflit textuel et il n’est pas compté comme un merge propre. `attempt-002-protected-red.log` conserve la panne réelle du vérificateur, tandis que l’assertion de classification du pilote attendait à tort le texte `min <= max`. Les tentatives suivantes ont corrigé ce détecteur; l’exécution finale reproductible est `attempt-004`.

## Limites et suite

Ce canari prouve que des tests propres à chaque branche peuvent être verts après un merge textuellement propre, tandis qu’un critère d’acceptation indépendant révèle une incompatibilité de comportement et permet une correction vérifiée. Il ne prouve pas que l’actuelle autorité Forgejo matérialise un merge preview et exécute l’acceptation sur celui-ci; le gate actuel refuse d’abord une base périmée. Il n’établit donc ni une PR réelle refusée pour `semantic_acceptance_failed`, ni une décision d’intégration automatique. La suite est de porter ce scénario dans la composition Forgejo locale avec la preuve liée à l’arbre combiné, en vérifiant le refus motivé et la PR non fusionnée. Aucun résultat de productivité ou comparaison n’est inféré.

Toolchain observée : Python 3.12.3, Git 2.43.0, rustc 1.95.0, Cargo 1.95.0; Linux aarch64. Toutes les dépendances Cargo sont nulles et les commandes utilisent `--offline --locked`.
