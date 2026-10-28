# MISSION-012 — refus d’un candidat modifié après vérification

Date : 25 octobre 2026. `HEAD` au démarrage : `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`; le worktree était fortement modifié, donc ce commit seul ne décrit pas les sources exécutées. Le snapshot de composition lie 83 fichiers par SHA-256 (`composition-source-snapshot.zip`, SHA-256 `a8f38015f8ac5936c8b247c2312abfdacac930c0c622430b3bae8f6ce56539da`). Les empreintes des sources de ce lot sont dans `source-sha256.txt`.

## Résultat observé

Le composeur utilise le local-test-adapter, une instance Forgejo locale et un webhook synthétique signé. Sur le commit de base `19798c1b08cd9d6177f252ac11e363ddc4ff3fe9`, le vérificateur protégé échoue sur l’acceptation de borne supérieure (`1` réussite, `1` échec; obtenu `-2`, attendu `10`). Le candidat `818bef52debf7697569fb52f6871797a0a8310c0` passe ensuite les deux critères, puis les deux critères passent de nouveau lors de la revérification pré-merge.

Après ces vérifications, le runner modifie via l’API Forgejo le seul fichier de la PR, `src/lib.rs`, pour rendre `maximum` égal à `maximum - 1`. La tête passe de `818bef52debf7697569fb52f6871797a0a8310c0` à `832e966d82b2479e72c21457063c1f61bdbbc832`; le SHA-256 de la source passe de `173304ad7d32aabc1e8e1ee41eabda3398a8690798e68f906bcebc73cbcd6d15` à `28248510f8d9c44539290bd3d4322befe6465fcc30381f2637dde85b1b27ddf1`. Un diagnostic séparé exécute cette nouvelle tête avec le même test d’acceptation protégé, dans Bubblewrap sans réseau, avec le snapshot en lecture seule, un `target` temporaire et des limites systemd/prlimit : une assertion passe et celle de la borne échoue (`9`, attendu `10`).

Le vrai processus local `forgejo-reconcile-local` reçoit ensuite l’ancienne preuve liée à la tête vérifiée. Il relit la PR modifiée et refuse avec `invalid_review_base_binding`, verdict `rejected`, sans autoriser ni tenter la fusion. Le readback Forgejo confirme que la PR reste ouverte/non fusionnée et que l’issue reste ouverte; `main` reste au commit de base et son fichier conserve le hash de la version initialement défectueuse. Le manifeste de composition et les reçus lient exigences, politique, candidats, binaire de vérification, réservations CPU et lecture après refus.

## Reproduction et contrôles

Les commandes sont dans `commands.txt`; les versions exactes sont dans `tool-versions.txt`. Lors du premier contrôle final, la suite a trouvé le répertoire d’admission conservé par ce parcours et a échoué avant de construire les étapes; le test du builder a ensuite reçu un nom UUID non réutilisable. La suite rerun passe `39/39`. Le diagnostic du candidat muté est une attente de refus : Cargo retourne `101` et le journal montre `1 passed; 1 failed`. Les manifestes locaux et global couvrent les sources, journaux, reçus, snapshot et ce rapport.

## Limites

Il s’agit d’un cas isolé (`n=1`) dans un bac à sable local avec Forgejo `17.0.0+gitea-1.22.0` et des identités/données synthétiques. Le binaire Forgejo utilisé est le binaire de release local identifié par son SHA-256; ce lot ne le reconstruit pas depuis le commit source audité. `local-test-adapter` n’a appelé aucun LLM; coût modèle rapporté `0 USD`, tokens indisponibles. AgentOS/Forge Code/RAG/mémoire ne sont pas intégrés ici. Ce test démontre le refus pour une preuve devenue périmée après mutation de tête; il ne prouve pas une comparaison de valeur, une résistance générale à tous les writers, un parcours LLM, ni un gain de productivité. Aucun code Forgejo n’est copié ou déployé et aucune instance de production n’est touchée.
