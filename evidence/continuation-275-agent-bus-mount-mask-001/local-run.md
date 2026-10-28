# CONTINUATION-275 — masquer le bus hôte au sandbox agent

Date : 2026-10-27  
Worktree : `.runtime/webhook-005-workspace`  
HEAD de base : `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0` (fortement modifié, aucun commit créé).

## Constat

Le composeur externe place son `/run/user/1000/bus` privé dans son namespace de montage afin d’appeler systemd. Le socket de cette machine existe, est possédé par l’UID 1000 et a le mode `0666` (`host-bus-readback.json`). `mission_012_sandboxed_test_adapter.py` créait ensuite un Bubblewrap imbriqué avec des tmpfs sur `/tmp` et `/home`, mais laissait `/run` hérité. Le fait de ne pas passer `DBUS_SESSION_BUS_ADDRESS` au processus évalué ne suffisait donc pas : le chemin absolu du socket restait dans son arbre de montages.

Un test de contrat rouge a montré que la commande `bubblewrap_command` ne masquait pas `/run`. La correction applique `--tmpfs /run` au processus agent. La sonde test-only `host_systemd_bus` fait un `lstat`, sans ouvrir ni contacter le socket; elle ne réussit que si le socket hôte connu présent est absent avec `ENOENT` dans le namespace agent. Le test de processus exécute également la sonde tandis que l’agent attend la séquence reprobe/libération, puis vérifie que l’événement gate ne change pas.

## Vérifications

- `python3 -m unittest -v tests.test_mission_012_sandboxed_test_adapter` : **12 réussis, 0 échec** (`sandbox-adapter-tests.log`). La sonde dynamique n’a pas été ignorée : le socket utilisateur était présent sur l’hôte.
- `python3 -m py_compile scripts/mission_012_sandboxed_test_adapter.py scripts/mission_012_test_agent_adapter.py tests/test_mission_012_sandboxed_test_adapter.py` : réussie.
- `git diff --check` : réussie.
- Bubblewrap 0.9.0; Python 3.12.3; Linux local.

## Limites

Cette tranche corrige le montage du sandbox du `local-test-adapter`; elle ne prouve pas la politique de tous les superviseurs, ni l’exclusivité du credential de fusion, ni l’absence d’autres chemins de privilège. Le composeur hôte conserve son bus pour le cycle de vie systemd. Le processus agent partage encore l’identité Unix dans cette fixture; la mesure masque un chemin D-Bus précis et ne le sépare pas de l’hôte. Aucun appel réseau au bus, Forgejo live, API/Web/`receive-pack`, modèle LLM, AgentOS ou benchmark n’a été effectué. Le test dynamique est conditionnel à la présence du socket, et le test statique du montage reste toujours actif.

État source et commandes : `source-sha256.txt`, `commands.txt`, `toolchain.txt`. Les anciennes conclusions liées au wrapper historique sont conservées comme preuves datées; [l’architecture](../../docs/architecture.md) les qualifie maintenant explicitement.
