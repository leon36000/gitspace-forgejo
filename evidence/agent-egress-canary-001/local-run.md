# AGENT-EXECUTION-EGRESS-CANARY-001 — primitive d’isolation locale

Date : 25 octobre 2026. Travail local uniquement dans le checkout dont le HEAD de base est `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`; l’arbre est fortement modifié et le test est un fichier de preuve non commité. Aucun fichier de production, credential réel, service inconnu ou modèle n’a été utilisé.

## Résultat

Le test [`test_bwrap_egress_canary.py`](test_bwrap_egress_canary.py) démarre un listener HTTP/TCP synthétique sur loopback et écrit temporairement un marqueur non secret dans `/tmp`. Le processus hôte le joint avec succès. Un sous-processus Bubblewrap (`--unshare-net --unshare-pid`) reçoit uniquement `/usr`, `/etc/alternatives`, `/dev` et `/proc`, avec des `tmpfs` privés pour `/tmp` et `/home` et un environnement nettoyé. Il ne peut pas joindre l’adresse loopback de la fixture et ne voit pas le marqueur hôte. Le marqueur synthétique est supprimé par le test après vérification.

Sortie : `host_fixture=reachable`, `sandbox_network=blocked`, `host_tmp_canary=hidden`, `external_network_used=false`; exit code 0. La commande complète et les versions sont dans [`commands.txt`](commands.txt) et [`toolchain.txt`](toolchain.txt); la sortie brute est [`local-run.log`](local-run.log).

## Interprétation et limites

- Ceci démontre le comportement de cette primitive Bubblewrap sur un enfant contrôlé, pas son intégration à un runtime d’agent. Aucun agent, Forgejo vivant ou serveur de modèle n’a été lancé.
- Le canari prouve l’inaccessibilité de la fixture IPv4 loopback testée et d’un seul chemin synthétique masqué dans `/tmp`. Il n’évalue pas les sockets Unix, les descripteurs hérités, les autres familles réseau, les attaques noyau ou les limites de même UID.
- Aucun secret réel n’a été monté ou lu. Ce test ne prouve donc pas qu’un futur runner n’exposera aucun credential.
- Le contrôle `MergeAuthority` existant est une allowlist dans le processus du client de gate; ses 5 tests précédemment archivés vérifient des refus de méthode/corps/chemin et une lecture autorisée sur fixture. Ce n’est pas une règle d’egress OS et ne prive pas un autre processus d’un accès réseau direct.
- L’inspection des scripts existants trouve Bubblewrap dans des étapes de build/vérification offline, mais aucune implémentation réelle d’`AgentRuntimeV1` dans `src`. La fixture d’agent est dans `tests/agent_runtime_contract.rs`.

Verdict : **primitive OS testée; séparation exécuteur / vérificateur / gate non démontrée.** Le prochain lot doit construire un adaptateur minimal qui applique la frontière au runtime existant ou au test-fixture, sans créer un autre moteur d’orchestration, et vérifier credential/egress avec données synthétiques.

Traçabilité : les six fichiers du manifeste local ont été vérifiés. Les sept entrées globales les plus récemment ajoutées ont été vérifiées également. Une entrée historique antérieure de `docs/checkpoint.md` ne correspond plus à la version actuelle car le checkpoint a reçu un autre ajout ensuite; elle reste dans le manifeste append-only, et l’empreinte actuelle a été ajoutée après elle puis vérifiée. Aucun historique n’a été réécrit.
