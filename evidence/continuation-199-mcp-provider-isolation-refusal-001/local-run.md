# CONTINUATION-199 — refus du chemin MCP Claude sans isolation attestée

Date d'exécution : 27 octobre 2026. Checkout `/home/pc6/Documents/Codex/2026-10-21-mission-construire-une-forge-con-ue/.runtime/webhook-005-workspace`, HEAD de base `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`, arbre préexistant fortement modifié. Linux/aarch64, Python 3.12.3. HEAD, environnement et empreintes de source : `toolchain-and-source-hashes.json`.

## Constat et changement

La lecture de `scripts/run_mission_012_live_mcp_agent_e2e.py` a montré que `build_agent_command` pouvait résoudre et lancer Claude directement depuis l'hôte; le rapport du runner identifiait alors `host_process_unisolated`. Le cgroup ajouté par CONT-197 garantit une durée et un nettoyage bornés, mais n'isole pas le réseau, le filesystem ou les credentials.

Le runner refuse maintenant `agent_adapter="claude"` dès l'entrée de `run`, avant toute résolution du répertoire de mission. Le builder refuse aussi Claude avant de chercher son exécutable. Le refus nomme les deux capacités manquantes : isolation filesystem et egress fournisseur attestées. Le chemin explicite `local-test-adapter` conserve sa route vers le wrapper Bubblewrap lorsqu'on demande `sandbox_test_adapter=True`.

Les deux nouveaux tests ont été lancés avant le correctif et ont échoué comme attendu : le builder tentait de résoudre Claude et `run` accédait au chemin d'état inexistant. Après correction, les cas passent.

## Vérifications

```text
python3 -m unittest tests/test_mission_012_live_mcp_agent_e2e.py -v
python3 -m unittest tests/test_mission_012_live_composed_e2e.py -v
python3 -m py_compile scripts/run_mission_012_live_mcp_agent_e2e.py tests/test_mission_012_live_mcp_agent_e2e.py
git diff --check
```

Résultats : **11/11** tests du runner MCP et **12/12** tests du runner composé réussis; compilation Python et diff-check réussis. Les journaux exacts sont `agent-runner-tests.log`, `composed-runner-tests.log`, `pycompile.log` et `diff-check.log`.

Aucun Claude CLI, processus de session Claude, modèle, Forgejo live, réseau externe ou coût n'a été utilisé. Les assertions de refus sont des tests locaux de code; le test de commande vérifie que le builder n'effectue pas la résolution du binaire, et le test `run` fournit un chemin inexistant pour détecter tout accès prématuré au state. La commande du test-adapter confirme encore le dispatch vers l'adaptateur sandboxé.

## Portée et suite

Ce garde s'applique à cet entrypoint MISSION-012 seulement. Il ne crée pas d'egress proxy et ne prouve pas que tous les entrypoints de tous les fournisseurs sont sûrs. Il bloque le parcours LLM ici jusqu'à une frontière vérifiable; il ne transforme pas le test-adapter en preuve d'un parcours LLM. Les parcours Forgejo/webhook déjà exécutés ne sont pas rejoués ni revendiqués comme nouveaux dans cette tranche.

Prochaine étape : inventorier les entrypoints d'exécution de modèles restants, puis décider à partir des interfaces locales constatées comment fournir l'egress modèle sans accès direct de l'agent aux routes Forgejo, credentials ou fichiers privés. Aucun coût ou nouvel accès n'est autorisé par cette preuve. La mission globale reste incomplète.
