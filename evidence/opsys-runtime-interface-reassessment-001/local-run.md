# OPSYS-RUNTIME-INTERFACE-REASSESSMENT-001

Date : 26 octobre 2026. Lecture locale en cours vers 21:16 UTC. Aucun fichier du dépôt OPSYS-AI n’a été modifié; aucun service n’a été lancé, aucune base ouverte, aucun `.env` lu, aucun secret consulté et aucun appel de modèle effectué.

## Source locale et versions examinées

Le dépôt `/home/pc6/Documents/Codex/2026-10-05/github-plugin-github-openai-curated-remote-4/work/OPSYS-AI` existe au commit `f0b89ccf86d6f6e50eb134de3257af91860e8091`, branche `codex/restructuration-2026-09-30`, 54 commits derrière sa branche amont. Le statut Git contrôlé exclut les fichiers non suivis (`--untracked-files=no`) et n’indique aucune modification suivie. `src/opsys/__version__` vaut `2.0.0.dev0`; `core/package.json` identifie OPSYS-CORE comme fork d’OpenClaw `2026.9.6`. Le verrou `uv.lock` fixe `mcp` et `mcp-types` à `2.2.0`. Les empreintes SHA-256 des sources lues sont dans `source-hashes.txt`.

## Interfaces exécutables dans le code

- `src/opsys/serveur_mcp.py::creer_serveur` expose les outils de capacités départementales via `tools/list` et `tools/call`. `pyproject.toml` déclare le binaire local `opsys-mcp`. Le dépôt contient les entrées `.venv/bin/opsys-mcp`, `.venv/bin/opsys-metier` et `.venv/bin/opsys-console`; aucune n’est trouvée par `command -v` dans le PATH courant.
- Le serveur MCP supporte stdio et HTTP streamable à `/mcp`. En HTTP, le code exige `OPSYS_MCP_JETON` d’au moins 24 caractères. Le mode stdio ne demande pas ce jeton; si aucun `--departement` n’est passé, la CLI construit le serveur sans filtre départemental. Cela expose une surface d’outils large si on la démarre par défaut; aucun serveur n’a été démarré. Le code permet de borner la sélection avec `--departement`, mais une intégration exigerait encore une politique de moindre privilège au niveau des outils.
- OPSYS-CORE possède des handlers Gateway `agent.run`/`agent.wait` et un flux conversationnel `chat.send`. Ce sont des chemins d’un runtime d’agent, distincts du serveur MCP métier. Leurs paramètres, autorisation, modèle effectif, admission de workspace et résultats n’ont pas été adaptés ni testés avec le contrat produit `AgentRuntimeV1`.

## Disponibilité locale et compatibilité avec la forge

Au snapshot `2026-10-26T21:16:49Z`, la table locale d’écoute n’indique ni `127.0.0.1:8765` (port HTTP documenté du MCP OPSYS), ni `127.0.0.1:18789` (port Gateway documenté dans `core/docs/install/docker.md`). Un listener existe sur `127.0.0.1:9119`, mais son propriétaire n’a pas été identifié et aucune requête n’y a été envoyée. Ce constat ponctuel ne démontre pas l’état d’un service distant ou d’un autre namespace.

Le contrat Forge `AgentRuntimeV1::execute(request, host_owned_tool_port)` reçoit des exigences et un contexte liés à leur source/version, un budget et un canal d’outils fourni par le host; il ne démarre pas lui-même un agent ni ne valide/fusionne son résultat. Aucun runtime OPSYS n’a été démontré comme implémentation compatible. Le MCP métier est un serveur de capacités, pas l’orchestrateur d’une mission logicielle. Décision : conserver `FixtureRuntime` comme adaptateur de test identifié; ne pas créer un second orchestrateur et ne pas brancher le Gateway ou les outils métier tant qu’un runtime autorisé, son endpoint, son modèle/coût et le mapping d’autorité n’ont pas été confirmés.

## Licences et limites d’examen

Le `LICENSE` à la racine OPSYS-AI commence par Apache License 2.0, mais `README.md` signale le conflit avec ADR-0003; ADR-0003 §Q7 indique « Licence propriétaire ». Le manifeste du paquet Python n’établit pas une décision qui résolve cette contradiction. Le `package.json` interne au core porte `MIT`, et `core/PROVENANCE.md` décrit un fork dur d’OpenClaw. Aucun avis juridique ni audit complet des dépendances, sous-modules, fichiers Enterprise ou droits de redistribution n’a été fait. Aucun code OPSYS-AI ou OpenClaw n’est copié dans la forge et aucune licence n’est choisie pour le produit.

Le manifeste Cargo du produit a été relu avec `cargo metadata --no-deps --format-version 1` : `agent-forge-core 0.1.0`, `license=null`, `license_file=null`, `publish=[]`. `cargo 1.95.0`, `rustc 1.95.0` et `git diff --check` (réussi) sont consignés dans `product-cargo-license-state.json`. Cela confirme l’absence de choix de licence, sans trancher les droits de redistribution.

Le contrôle est une inspection ciblée des manifests, de l’interface MCP et des handlers Gateway avec leurs fichiers hashés; aucun test OPSYS, compilation, service, modèle, base métier, route HTTP, credential ou comportement d’agent n’a été exécuté. Il établit l’existence de sources et d’un contrat, pas une intégration ou une disponibilité opérationnelle.

## Intégrité du reçu

Les sources et documents de forge sont liés par les manifestes locaux, du worktree et du projet; `manifest-prefix-before.json` fixe leurs préfixes préexistants. La première exécution du nouveau script append-only a correctement préservé les préfixes locaux et du worktree, puis échoué avant d’écrire dans le manifeste projet à cause d’une écriture bytes/texte. L’erreur exacte est conservée dans `manifest-helper-initial-error.txt`; le script a été corrigé et les trois index ont ensuite été vérifiés indépendamment. Aucun ancien octet d’index n’a été modifié.
