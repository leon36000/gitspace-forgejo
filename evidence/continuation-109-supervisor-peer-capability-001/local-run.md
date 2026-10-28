# CONTINUATION-109 — faisabilité d’un canal de capacité superviseur

Date : 26 octobre 2026. HEAD inspecté : `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. Cette sonde locale n’a pas démarré de Forgejo, gate, agent, receiver ou listener réseau; aucun credential n’a été créé ou lu.

## Expérience

Le script [`probe_supervisor_socket_capability.py`](probe_supervisor_socket_capability.py) crée une paire AF_UNIX `SOCK_SEQPACKET` dans le superviseur hôte, transmet un seul FD à Bubblewrap 0.9.0 sous UID 1000 et échange une trame bornée `permit-once`. Python est 3.12.3. Deux cas ont été exécutés dans un namespace réseau distinct :

1. Sans `--unshare-pid`, le child partage le namespace PID hôte. `SO_PEERCRED` retourne le PID du superviseur créateur `1642817` et UID `1000`. Le child, PID `1642821`, a `PPID=1642820`; le parent Bwrap est `1642820`. L’inode réseau enfant `net:[4026535808]` diffère de celui de l’hôte `net:[4026531833]` et correspond au readback host de `/proc/1642821/ns/net`. Le paquet unique est reçu et confirmé; l’enfant sort code 0.
2. Avec `--unshare-pid`, le child voit PID `2`, PPID `1`; `SO_PEERCRED` traduit le PID du superviseur en `0`. Le namespace PID est `pid:[4026535951]`, différent de l’hôte `pid:[4026531836]`. Le même paquet est transmis et confirmé; l’enfant sort code 0.

La sortie machine lisible est [`probe-result.json`](probe-result.json); le SHA-256 du script et des résultats est dans [`source-sha256.json`](source-sha256.json).

## Décision de conception

Pour une future admission locale, un socketpair possédé par le superviseur peut servir de capacité vivante et à usage unique. Le gate devra s’exécuter avec Forgejo dans le même namespace réseau privé et sans `--unshare-pid`, afin de pouvoir vérifier le PID pair noyau du superviseur et contrôler dans `/proc` la durée de vie et les namespaces actuels de Forgejo et du gate. L’agent reste isolé dans son propre namespace réseau **et** PID; le canal et le credential ne lui sont pas transmis.

La trame future devra être strictement bornée et liée au dépôt, à l’origine, au PR, aux PID/temps de démarrage du gate et de Forgejo, à leurs namespaces, à une expiration brève et à un nonce. Le gate doit la consommer avant de lire son credential. Le superviseur doit garder l’autre extrémité ouverte jusqu’à la décision et fermer/révoquer en cas d’arrêt ou de dérive. `SO_PEERCRED` établit l’identité noyau du créateur du socket, pas la correction de sa politique; la sécurité dépend de l’implémentation du superviseur et exclut les acteurs root/privilégiés et le noyau compromis.

## Vérifications et limites

Les deux cas ont respecté leurs assertions, livré une trame bornée sur le FD, gardé le child vivant jusqu’à l’observation puis terminé code 0. La commande exacte et son code de sortie sont dans `command.txt`. Le script lui-même porte les assertions sur peer PID/UID, namespaces et identité `/proc`.

Ce n’est pas un reçu de fusion positif. Aucun vrai Forgejo, autorité gate, PR, credential, webhook ou processus agent n’est inclus; aucune route Forgejo ni exclusion d’écrivains n’est testée. Le paquet `permit-once` est seulement un handshake d’essai, pas un protocole de reprise, expiration, rejeu, fermeture ou annulation. L’isolation de l’agent et son absence de descripteur sont couvertes par des preuves antérieures distinctes, pas par cette sonde. La CLI reste refusée tant que l’interface typée n’est pas intégrée et qu’un parcours composé ne prouve pas la provenance.

Prochaine action : implémenter le lecteur de capacité dans le vrai gate et ajouter les refus de trame manquante/malformée, peer PID/UID incorrect, cible, PID/namespace et expiration périmés, paquet répété, fermeture du superviseur avant POST et absence de consommation du credential; ensuite lancer un parcours local Forgejo + gate sous ce superviseur avec l’agent séparément isolé. Aucun appel de modèle ou dépense nouvelle n’est requis.
