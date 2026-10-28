# FORGEJO-STDIO-MCP-SAME-CANDIDATE-001 — source MCP liée à la PR fusionnée

Date : 26 octobre 2026. Exécution locale dans le worktree fortement modifié, branche local/WEBHOOK-005-canonical-links, HEAD de base f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0. L’instance Forgejo, ses utilisateurs synthétiques, ses tokens et son dépôt restent sous /tmp/forgejo-mcp-same-source-003; le dossier de preuve ne contient aucun token ni verifier.key.

## Résultat observé

La commande complète est dans commands.txt. Forgejo 17.0.0+gitea-1.22.0 a servi le dépôt local. Le même processus --stdio-mcp a consommé le contexte, observé un échec Cargo actionnable, corrigé src/lib.rs via workspace_write_text, repassé l’acceptation puis attendu à la barrière du gate.

Le runner a lu les octets de ce workspace pendant que le processus attendait. Leur SHA-256 78cc4a0a2cafa1d15b62adc8782af05445e1708268d618bf970de3b2340d407a correspond à celui du readback de la branche candidate, du commit de tête final de la PR #2, du code à main et du readback après fusion. Le commit candidat final est 968679a65ee22482bb1ebe800628ac096af13a4d; le commit de fusion et la tête main sont 32f1a8cc3a821f02e7b8e82412e652ba9589e34b. La preuve de liaison est dans stdio-mcp-source-binding.json; les snapshots sont dans source/.

Le processus isolé est resté vivant jusqu’à l’opération du gate. Ses sondes directes API, Web et git-receive-pack ont toutes échoué dans son namespace réseau; le superviseur a observé zéro connexion de route. Le processus n’avait que les FD 0, 1, 2, aucun credential visible ni transmis, et le probe /proc du memfd CLOEXEC du gate a été refusé dans son namespace PID. Pendant ce temps, la lecture Forgejo de confiance a répondu HTTP 200 et le gate Rust non-admin a fusionné la PR en HTTP 200. Les résultats sont dans stdio-mcp-egress-during-merge.json.

La suite indépendante a d’abord refusé le candidat initial avec cinq cas rouges et quatre verts. Après la correction MCP, le vérificateur de la PR a passé 9/9; le readback après fusion a testé le hash source exact et passé 9/9. Le runner refuse également le faux succès de l’exécutant, un gate sans revue autorisée, un reviewer non autorisé et un merger non autorisé. Le readback Forgejo confirme PR fermée/fusionnée, issue fermée et main == merge_commit_sha.

L’acceptation MCP est un cas de bord supérieur; le vérificateur indépendant couvre les cas minimum/maximum, extrêmes i32, intervalle singleton et valeurs incluses. Le contexte d’acceptation reste protégé côté hôte, pas fourni par le processus évalué.

## Diagnostic et correction du run

Les essais 001 et 002 ont refusé en mode fail-closed au préflight final (not_mergeable). La trace 002 montre la tête PR et la base attendues mais mergeable=false. Forgejo v17 marque une PR Checking lors de son ajout à la file de recalcul (services/pull/check.go::AddToTaskQueue), puis le worker la fait passer à Mergeable après ses contrôles (checkAndUpdateStatus). Dans le run 003, le readback après restauration a montré true, false, false, true, true, true pour la même tête en environ 1,38 s; le premier true était donc périmé.

Le runner attend maintenant une observation stable true après la transition asynchrone, ou 3 secondes de stabilité si le false transitoire n’est pas observé. Un test unitaire garantit qu’il ne prend pas le premier true pour résultat final. Après le probe qui invalide l’ancienne preuve, le runner restaure les octets MCP exacts dans la branche PR, vérifie le nouveau head, puis refait la vérification indépendante et la preuve liée avant la fusion.

L’historique de ces essais et leurs diagnostics sont conservés dans attempt-history.md et attempt-002-final-preflight-state.json.

## Vérifications fraîches

- Exécution complète Forgejo locale avec le processus MCP tenu pendant la vérification, la revue et le gate : code 0; PR #2 fusionnée, issue fermée, validation post-fusion 9/9.
- Source MCP, source de candidat, source fusionnée et source lue sur main : SHA-256 identique (78cc…407a).
- Poll de recalcul Forgejo : 6 observations; false → true stable après recalcul.
- Tests ciblés Python des adaptateurs MCP, du sandbox et du poll mergeabilité : 11/11.
- python3 -m unittest discover -s tests -p 'test_*.py' -v : 242 réussis, 0 échec.
- python3 -m py_compile sur les scripts et tests touchés : code 0; git diff --check : code 0.
- Versions, commande, empreintes du runner, preuves sélectionnées et toolchain : provenance.json, source-hashes.json, toolchain.txt.

## Portée et limites

--stdio-mcp utilise le local-test-adapter déterministe; aucun LLM, AgentOS, Forge Code, RAG ou moteur de mémoire n’a été appelé. Ce résultat est n=1 et ne mesure ni gain comparatif, ni coût tokens, ni correction générale. Aucun événement webhook n’a été livré/réconcilié dans cette même exécution. Le parcours webhook distinct MISSION-012-COMPOSE-064 reste une preuve complémentaire sur une autre exécution; il ne doit pas être présenté comme étant lié à ces octets MCP.

La frontière repose sur Bubblewrap, les namespaces et la fermeture des FD dans cette configuration locale. Forgejo et le runner partagent l’hôte et l’UID; aucune garantie contre un noyau compromis, un acteur privilégié ou une évasion n’est revendiquée. external_writers_excluded=false: les writers Forgejo externes ne sont pas exclus et cette preuve n’établit pas une précondition CAS serveur universelle. Le test ne couvre pas toutes les routes Forgejo.

Le code du gate est un profil local de test et l’instance est jetable; aucun code Forgejo/GitSpace n’a été copié ni modifié. Aucun service externe, coût modèle, secret externe, publication ou environnement de production n’a été utilisé. Le projet reste incomplet.
