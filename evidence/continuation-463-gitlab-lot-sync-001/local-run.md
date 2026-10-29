# CONTINUATION-463 — Readback du transfert CONT462 sur GitLab PC4

Date : 29 octobre 2026. Exécution depuis le worktree PC6, projet GitLab privé `root/gitspace-forgejo` (ID 4), branche `codex/pc6-transfer-work-2026-10-28`.

## Opération

Le lot CONT462 a été appliqué par l’API GitLab dans le commit `4dc2c2d7c61c81426c79b2596cc4188529b8b102` (`fix: bind present comments to fresh parent [skip ci]`). L’appel contenait 24 actions : 15 remplacements dans les fichiers source, tests, checkpoint et manifeste, puis neuf créations de fichiers de preuve. L’appel a renvoyé `errors=[]`.

## Vérification distante

- Avant le commit, la branche de travail pointait sur `27a6eda885d578203d5c9f234ceb7da1110208cd`, était non protégée et `can_push=true`; la branche par défaut protégée pointait sur `6c6ffa3d7d37d48b9d7877acb982d73a2c49a14f`.
- Après le commit, la branche de travail pointe sur le SHA ci-dessus; la branche par défaut reste inchangée et protégée.
- `list_repository_tree` a confirmé les IDs d’objets Git et modes `100644` locaux pour les trois fichiers source/test, le checkpoint et les neuf fichiers de preuve. `evidence/SHA256SUMS` a aussi le même ID d’objet Git local et distant.
- Le manifeste global passe de 19 701 à 19 716 lignes. Le préfixe distant de 19 701 lignes a le SHA-256 `2f872fa51eb0271adb8c5087687d4570066f73673324ba08881b34fff5438efc`; le suffixe distant de 15 lignes est identique au suffixe local préparé.

Les IDs complets, modes et commandes locales sont consignés dans `remote-readback.json` et `commands.txt`.

## Portée et limites

Les vérifications CONT462 restent celles exécutées sur PC6 : contrat réconciliateur 115/115, quatre scénarios processus sous Bubblewrap 4/4, `cargo fmt --all -- --check` et `git diff --check`. Elles utilisent des fixtures HTTP synthétiques; aucun Forgejo live n’est revendiqué.

Ce transfert transporte un instantané de fichiers texte, pas les commits/historique Git PC6. Aucun shell/runner PC4 ni pipeline, MR ou merge n’a été utilisé. Les archives ZIP et le bundle du dépôt imbriqué sont toujours absents de la branche. Aucun secret de courriel n’a été lu ou transféré. Le parcours LLM/AgentOS et le benchmark comparatif restent ouverts.
