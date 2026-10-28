# AGENT-FD-BOUNDARY-001 — descripteurs réellement visibles par l’adaptateur MCP

Date : 25 octobre 2026 (America/Montreal). Worktree fortement modifié, base `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. Cette tranche ne lance aucun modèle et n’utilise que des fixtures locales.

## Lacune et changement

Le parcours [MISSION-012-SANDBOX-STDIO-001](../mission-012-live-agent-webhook-local-adapter-sandbox-001/local-run.md) avait relié l’adaptateur Bubblewrap à un vrai hôte MCP de confiance par stdin/stdout, fermé les copies de pipes et utilisé `close_fds=True` sans `pass_fds`. Le contrôle de FDs `[0, 1, 2]`, toutefois, était observé dans un processus probe séparé, pas dans le processus qui exécutait l’adaptateur.

J’ai d’abord modifié le test du vrai dialogue stdio pour exiger `sandbox_checks.open_fds == [0, 1, 2]`. Le test a échoué comme prévu : le résultat du processus adaptateur ne rapportait pas cette propriété. Le fixture énumère maintenant les FD ouverts via `/proc/self/fd`, en excluant le FD temporaire utilisé pour lire `/proc`; son propre probe produit la liste et le wrapper vérifie qu’elle est exactement `[0, 1, 2]`. Le probe tenu ouvert utilise le même échantillonnage. Le runner MCP et le runner de merge attendent eux aussi cette preuve exacte.

## Vérifications

- Tests ciblés des contrats sandbox, runner MCP et runner d’egress : **19/19 réussis**.
- Suite Python : **231 réussis, 0 échec**.
- `ast.parse` des cinq scripts/tests modifiés : réussi.
- Le test d’intégration démarre le wrapper réel, le vrai processus adaptateur et un faux hôte MCP limité aux quatre outils; celui-ci vérifie le contexte, l’échec rouge, la correction, le test vert, l’interruption après écriture, l’absence du canari de fichier et le refus des nouvelles connexions loopback.
- Journaux et codes de sortie : `focused-tests.log`, `python-tests-all.log` et leurs fichiers `.exit-code`. Les empreintes sources sont dans `source-hashes.txt`.

## Portée et limites

Cette preuve établit `[0, 1, 2]` pour le processus Bubblewrap de l’adaptateur au moment de la sonde et fait échouer le wrapper si la liste diffère. Elle complète le canari de socket hérité et le test de routes Forgejo réelles déjà archivés; elle ne les répète pas. Elle ne prouve pas qu’aucun FD ne sera ouvert plus tard, ne constitue pas une isolation contre le même UID ou un noyau compromis, et ne démontre pas un runtime AgentOS/LLM. Le faux hôte MCP est une fixture; la composition Forgejo complète n’a pas été rejouée. Aucun code Forgejo, production, service externe ou dépense nouvelle n’est impliqué.

Le projet reste incomplet : la comparaison contrôlée agent+CI n’est pas exécutée et la limite de coût fournisseur demeure non fiable.
