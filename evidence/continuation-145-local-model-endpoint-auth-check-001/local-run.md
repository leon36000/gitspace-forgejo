# CONTINUATION-145 — vérifier l’accès au modèle local sans utiliser de clé

Date : 26 octobre 2026. Worktree `/home/pc6/Documents/Codex/2026-10-21-mission-construire-une-forge-con-ue/.runtime/webhook-005-workspace`, branche `local/WEBHOOK-005-canonical-links`, HEAD observé `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. Le dépôt est déjà fortement modifié; cette vérification documentaire n’a pas modifié de code ou de configuration d’exécution.

## Observation

La liste locale des sockets associe le processus `llama-server` à `127.0.0.1:18881`. La lecture sans authentification de `GET /health` retourne HTTP 200 et `{"status":"ok"}`. `GET /v1/models` retourne HTTP 401 avec `authentication_error` / `Invalid API Key`. Les binaires `ollama`, `llama-server`, `llamafile` et `vllm` ne sont pas trouvés dans `PATH`; un serveur `llama-server` déjà lancé reste visible comme listener local.

Recherche limitée à `src/`, `tests/`, `Cargo.toml` et `Cargo.lock` : le seul `impl AgentRuntimeV1` est `tests/agent_runtime_contract.rs::FixtureRuntime`. Le texte d’usage `mission-012-ollama-local` dans `src/bin/mission-012-verify-local.rs` ne constitue pas une intégration d’exécution de modèle. Aucun fournisseur réel n’est donc intégré à l’interface AgentRuntime du dépôt.

Ce résultat ne prouve ni le modèle servi, ni les limites d’usage du service, ni le droit d’utiliser sa clé. Aucun argument, environnement, log, historique de requête, clé ou session Claude n’a été lu; aucune clé n’a été cherchée, copiée ou transmise. Aucune requête de génération/inférence n’a été faite. Il n’y a eu ni coût modèle, ni accès étendu, ni envoi de données de projet hors de la machine. Le `/health` public ne suffit pas à prouver que l’API de modèles est autorisée. La fixture reste l’adaptateur vérifiable pour les contrats; utiliser un LLM réel exigera un accès local explicitement autorisé.

## Portée

Audit read-only de deux routes HTTP locales et du code d’interface Rust sous `src/` et `tests/`; aucun test logiciel ou parcours agent n’a été exécuté. La présence et l’authentification du service peuvent changer. L’inventaire n’exclut pas un autre runtime hors de ce dépôt ou une source d’autorisation non visible ici. Le benchmark contrôlé, l’exécution LLM réelle et l’intégration AgentOS restent incomplets.

Commandes, sorties, révision, empreintes et manifestes : [commands.txt](commands.txt), [health.log](health.log), [models.log](models.log), [listener.log](listener.log), [runtime-search.log](runtime-search.log), [model-command-paths.log](model-command-paths.log), [revision.txt](revision.txt), [source-sha256.txt](source-sha256.txt) et [cargo-license-current.json](cargo-license-current.json).
