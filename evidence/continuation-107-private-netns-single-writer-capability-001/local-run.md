# CONTINUATION-107 — capacité de namespace réseau privé sur l’hôte local

Date : 26 octobre 2026. Base Git inspectée : `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. Worktree fortement modifié; aucune modification, suppression, publication ou action de production n’a été effectuée dans cette sonde.

## Essai

`probe_private_loopback.py` lance un enfant Bubblewrap 0.9.0 sous UID 1000 avec `--unshare-all --unshare-user --disable-userns --assert-userns-disabled --die-with-parent --new-session --unshare-pid`. Le child démarre un petit serveur TCP uniquement sur son `127.0.0.1`, attend le signal du parent, puis s’y connecte lui-même. Pendant que le serveur est vivant, le processus hôte essaie le même port `127.0.0.1`.

## Résultat

Le serveur interne a répondu `private-netns-ok:probe`; la connexion depuis l’hôte a été refusée avec `ConnectionRefusedError`, errno 111, sur le port éphémère 54925. L’enfant est sorti avec code 0 et stderr vide. Sortie brute : `probe-result.json`; code : `probe-exit-code.txt`; source exécutable : `probe_private_loopback.py`.

Les premières tentatives de préparation sont consignées dans `harness-corrections.md`; elles n’ont pas démarré le serveur. La sonde finale reprend les options Bubblewrap déjà présentes dans le runner local et n’exige aucune capacité privilégiée supplémentaire.

## Portée

Cette expérience montre qu’un processus non privilégié peut maintenir un service loopback à l’intérieur d’un namespace réseau privé qui n’est pas joignable sur le loopback de l’hôte. Elle ne lance ni Forgejo ni le gate, ne teste pas les routes API/Web/`receive-pack`, n’utilise aucun credential et n’éprouve pas les voies d’entrée d’un autre processus privilégié ou d’une évasion noyau. Elle ne suffit donc pas à attester l’exclusion de tous les writers. Le profil `local_single_writer_sandbox` demeure refusé par CONTINUATION-106.

Prochaine action : exécuter le Forgejo local et le gate dans la même frontière, attester l’appartenance de leurs processus au même namespace, vérifier le readback interne et les probes hôte, puis seulement concevoir le format/provenance de l’attestation. Aucun CAS côté Forgejo ni aucun succès de fusion n’est revendiqué.
