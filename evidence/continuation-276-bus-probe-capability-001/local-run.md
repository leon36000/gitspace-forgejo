# CONTINUATION-276 — attester l’absence du bus dans le contrat sandbox

Date : 2026-10-27  
Worktree : `.runtime/webhook-005-workspace`  
HEAD de base : `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0` (dépôt fortement modifié, aucun commit créé).

## Changement

CONT-275 cachait `/run` au `local-test-adapter`, mais le champ de sortie de la sonde ne disait pas explicitement si le bus hôte était absent. Le test adapter parcourt maintenant `/run/user/*/bus` : un chemin présent ou une observation de métadonnées indécidable fait échouer le probe; seul l’absence (`ENOENT`) donne `host_systemd_bus_visible=false`. Le wrapper MCP exige ce champ avant d’accepter les événements de l’agent.

Le runner egress autonome utilise lui aussi `--tmpfs /run`, inclut le booléen dans son reçu schéma v3 et exige `false`. Les reçus historiques v1 et v2 restent acceptés selon les observations disponibles dans leur version; ils ne sont pas promus rétrospectivement en reçus v3.

Les tests rouges avant implémentation montraient que la réponse processus n’incluait pas le champ et que le validateur acceptait un reçu v3 qui déclarait le bus visible ou omettait la mesure (`red-test-observation.md`).

## Vérifications

- `python3 -m unittest -v tests.test_mission_012_sandboxed_test_adapter tests.test_forgejo_agent_egress_live_e2e` : **27 réussis, 0 échec** (`targeted-tests.log`).
- Le test de processus du probe egress s’exécute dans Bubblewrap alors que le socket D-Bus hôte est présent; il vérifie que le sandbox ne voit ni ce chemin ni le credential canari et qu’il ne peut pas joindre le listener loopback de l’hôte. Le test du sandbox MCP garde aussi l’agent en attente durant la reprobe/libération.
- Les tests du validateur refusent les reçus v3 dont le bus est visible ou non mesuré; les fixtures v1/v2 restent lisibles.
- `py_compile` sur les cinq sources Python touchées et `git diff --check` : réussis.
- Environnement : Python 3.12.3, Bubblewrap 0.9.0, Linux local.

## Limites

Le probe egress de ce lot n’a pas lancé Forgejo : son listener API est une cible loopback sans service, observée depuis un namespace réseau distinct. Aucun accès ou appel n’a été fait au socket D-Bus. Aucun processus LLM/AgentOS, credential de fusion, writer Forgejo, PR ou merge n’a été exécuté. La réponse atteste l’absence de chemins `/run/user/*/bus` dans les processus locaux sondés, mais pas une politique indépendante du superviseur, l’isolation de l’identité Unix, ni l’exclusion de tous les writers/voies d’évasion. Le partage d’UID et l’absence d’un LLM autorisé restent des limites matérielles.

Cargo reste `license=null`, `license_file=null`, `publish=[]`, sans choix de licence ni décision sur les droits. Les commandes et versions sont dans `commands.txt` et `toolchain.txt`; les sources sont hachées dans `source-sha256.txt`. Le contrôle positif sur le socket hôte est archivé dans [CONTINUATION-275](../continuation-275-agent-bus-mount-mask-001/host-bus-readback.json).
