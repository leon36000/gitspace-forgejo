# AGENT-EXECUTION-EGRESS-CANARY-002 — descripteurs hérités

Date : 25 octobre 2026. Exécution locale dans le worktree fortement modifié basé sur `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. Le seul credential est un marqueur synthétique en mémoire; la seule destination réseau est un listener TCP loopback éphémère. Aucun endpoint externe, processus préexistant ou secret réel n’a été utilisé.

## Résultat observé

Le test lance le même enfant Bubblewrap avec un descripteur memfd synthétique et une socket TCP loopback déjà connectée. Lorsque le parent passe explicitement le FD à `bwrap`, l’enfant lit le marqueur et envoie des octets au listener malgré `--unshare-net`. Lorsque le parent exécute `bwrap` avec `close_fds=True` et sans `pass_fds`, le marqueur ne devient pas visible et la socket ne livre aucun octet au listener.

Sortie : `explicit_fd_credential=visible`, `close_fds_credential=not_visible`, `explicit_fd_socket=reachable_across_unshare_net`, `close_fds_socket=not_delivered`, `external_network_used=false`. La commande et la sortie sont dans [`commands.txt`](commands.txt) et [`local-run.log`](local-run.log); versions et plateforme dans [`toolchain.txt`](toolchain.txt).

## Interprétation

Un namespace réseau bloque les nouvelles connexions testées, mais ne révoque pas un socket déjà ouvert et transmis au processus. La visibilité d’un credential dépend aussi de la politique d’héritage des FD du lanceur. Pour le canari étudié, une fermeture de FD dans le processus parent avant exec et l’absence de `pass_fds` empêchent ces deux transferts. Le contrôle appartient au lanceur et fait partie de la frontière de capacité; ce n’est pas une propriété autonome de Bubblewrap.

Le runner de test `scripts/run_mission_012_live_mcp_agent_e2e.py::build_agent_command` lance `scripts/mission_012_test_agent_adapter.py`, dont `MCPClient` démarre ensuite l’hôte MCP avec `close_fds=True`. Ce run canari ne modifie pas ni ne démontre l’isolation de ce parcours d’agent : l’adaptateur actuel consomme un fichier de configuration et exécute dans le contexte hôte. Aucun adaptateur d’agent de production ni AgentOS n’est présent dans `src`.

## Limites

Le test n’exerce que le passage explicite d’un memfd et d’un socket TCP connecté, puis leur absence via `close_fds=True`. Il ne couvre pas socket Unix, stdin/stdout/stderr volontairement raccordés, descripteurs créés après le lancement, accès noyau, attaque par même UID, identité ou capacités Linux, ni un véritable processus d’agent. La vérification « non visible » porte uniquement sur les octets synthétiques attendus; elle n’est pas une preuve d’absence de tout accès sensible.

Verdict : **la primitive réseau précédente reste valable pour les nouvelles connexions testées; l’isolation exige aussi que le lanceur garde une liste fermée des descripteurs transmis. Le parcours local-test-adapter n’est pas encore exécuté derrière cette frontière.**
