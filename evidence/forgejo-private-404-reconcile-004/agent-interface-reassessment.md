# Réexamen des interfaces locales d'agent — 25 octobre 2026

## Contrat de la forge

Le seul point d'extension visible dans le dépôt est `AgentRuntimeV1::execute` dans `src/agent_runtime.rs:4492` : requête d'exécution versionnée et liée à une mission, exigences hashées et port d'outils explicite. La seule implémentation trouvée par `rg -n "impl AgentRuntimeV1" src tests` est `FixtureRuntime` dans `tests/agent_runtime_contract.rs:46-50`; elle accepte une unique tâche de test déterministe. Ce contrat n'accorde aucune autorité de vérification ou de merge. Aucun exécuteur AgentOS/Forge Code réel n'apparaît dans `src/`.

## Source RAG locale constatée

Un arbre local nommé `pc5-h2-candidate` existe à `/home/pc6/Documents/Codex/2026-10-18-voici-l-ip-de-pc5-je/pc5-h2-candidate`. Son HEAD Git est `bd4dbca60d1a2fa5b0560d2591282b2114e8d610`, mais `git status --porcelain | wc -l` donne 61 entrées : ce n'est pas un checkout propre et son HEAD ne suffit pas à identifier les fichiers observés. Son manifeste `pyproject.toml` déclare `nvidia_rag` version `2.6.0.rc1`.

Dans `src/nvidia_rag/pc5/api.py`, le routeur FastAPI `APIRouter(prefix="/pc5")` expose `GET /sources/{doc_id}/{source_sha}`, `PATCH /documents/{doc_id}/access` (réservé au rôle admin) et `GET /capabilities`. `src/nvidia_rag/rag_server/main.py` déclare la classe de bibliothèque Python `NvidiaRAG` et ses opérations generate/search/summary/health. Ce sont des interfaces de récupération, génération et accès à des sources; les fichiers inspectés ne démontrent pas un protocole d'exécution de tâches de code compatible avec `AgentRuntimeV1`, ni un déploiement vivant, ni un endpoint effectivement autorisé pour cette forge.

Références vérifiées : `pyproject.toml` SHA-256 `2e93473b6e3514bf090a858d6a540b0010516ac98ed8363e25719e1a7b74ddfe`; `src/nvidia_rag/pc5/api.py` SHA-256 `5d69dc6663d48bff801ca6167a52298f9abc30454d391268cbe0b7ae34d411d2`; `src/nvidia_rag/rag_server/main.py` SHA-256 `32b2b5b4b49fa330e8e5635ffed0ef6cdbe6418b502cdf81117f807b88a0ee8d`; `README.md` SHA-256 `7c5f6422030dac85693e7e87d65b866de563934e664114d246e2f4d62546640d`. Le script `src/nvidia_rag/pc5/api.py` est marqué SPDX Apache-2.0; cet audit ne détermine pas les droits de l'arbre complet ni ne copie son code.

## Décision de cette tranche

Aucun appel à ce RAG, à un modèle, à AgentOS ou à Forge Code n'a été fait; aucun secret, donnée privée, réseau externe ou budget n'a été utilisé. L'absence de répertoire nommé AgentOS dans la recherche locale bornée n'est pas une preuve universelle d'absence. Pour le parcours testé, le fixture déterministe est explicitement conservé comme fixture; aucune intégration réelle n'est revendiquée. Le RAG demeure un candidat à un futur adaptateur de recherche/contextes sourcés, après vérification de son déploiement, de sa politique d'accès et d'un contrat d'appel autorisé. Il ne remplace pas un runtime de code et la forge ne duplique pas sa mémoire ou son RAG.
