# CONTINUATION-216 — diagnostic de l’identité des binaires dans Bubblewrap

Date : 27 octobre 2026. Worktree `.runtime/webhook-005-workspace`, HEAD examiné `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`; arbre de travail fortement modifié et préservé. Question : pourquoi l’admission du composeur courant refuse-t-elle `systemd-run/systemctl` quand la commande est lancée dans Bubblewrap?

## Source et frontière examinées

Dans `scripts/systemd_process_scope.py`, `_trusted_systemd_binary` exige un fichier régulier, `st_uid == 0` et aucun bit d’écriture groupe/monde. `prepare_process_scope` vérifie ces deux binaires avant `_runtime_and_bus`, puis ne fait que construire une commande; `launch_scoped_process` est la frontière qui démarre effectivement le scope. `scripts/mission_012_sandboxed_test_adapter.py::bubblewrap_command` monte `/usr` en lecture seule, utilise des tmpfs pour `/tmp` et `/home`, crée des namespaces réseau/PID et ne monte pas `/run/user`. `run_mission_012_live_composed_e2e.py::run` refuse aussi que le composeur partage le namespace réseau hôte. Les empreintes des trois fichiers observés figurent dans `source-sha256.txt`.

## Résultats

| Environnement | Observation | Verdict |
|---|---|---|
| Hôte, `prepare_process_scope(['/usr/bin/true'], ...)` | `host_prepare=ok` | Validation et construction seulement; aucune commande `systemd-run` n’est exécutée. |
| Bubblewrap de l’adaptateur (`/usr` bindé en lecture seule) | `/usr/bin/systemd-run` et `systemctl` visibles avec UID `65534`; `_trusted_systemd_binary=false` pour chacun | `prepare_process_scope` refuse avec `trusted systemd-run/systemctl binaries are unavailable`. |
| Forme outer Bubblewrap du run historique 121 (`/` bindé en lecture seule) | Les propriétaires restent `65534`; le socket bus est visible; même refus avant tout accès au bus | Rendre le socket visible ne rétablit pas l’identité root des exécutables. Aucun échange D-Bus n’a été tenté. |
| `unshare --net` sans user namespace | `unshare failed: Operation not permitted` | L’utilisateur courant ne peut pas créer cette namespace directement. |
| Service utilisateur transitoire avec `PrivateNetwork=yes` | La commande se termine avec succès, mais son namespace observé est `net:[4026531833]`, identique à celui de l’appelant; les propriétaires restent `65534` | Cette configuration n’a pas fourni de namespace réseau distinct ni rendu les binaires attestables. Le probe n’a lancé qu’un interpréteur Python de diagnostic. |

Les messages `trusted ... binaries are unavailable` sont donc trompeurs dans ce cas : les fichiers sont présents, mais leur UID root est non mappé par le user namespace. Si cette vérification était contournée, le wrapper de l’adaptateur n’aurait toujours pas le socket `/run/user/<uid>/bus`; ce n’est pas une raison de l’y monter.

## Vérifications et portée

Hôte : Python 3.12.3; Bubblewrap 0.9.0; systemd 255. Le service `systemd-run` était transitoire, limité à 15 secondes, `--collect`, et a terminé en 26 ms. Aucun scope de candidat, gate, Forgejo, PR, merge, modèle, credential ou service externe n’a été utilisé. Un premier mini-harnais `importlib` a échoué avant l’import du helper faute d’enregistrement du module dans `sys.modules`; il a été remplacé par un import direct via `sys.path`, sans exécuter de code produit ni modifier de fichier.

Ce lot établit le prérequis environnemental qui bloque le test du gate courant, pas une fusion, une preuve de réseau exhaustive ou une propriété de sécurité universelle. Il ne justifie ni de retirer le contrôle root-owned, ni d’exposer le bus au bac agent, ni de transformer la séparation en règle côté client. La suite utile est un lancement contrôlé par un composant de confiance qui peut créer/posséder le namespace et le scope tout en laissant le bac agent sans bus; le protocole de handoff, l’isolement de l’exécuteur et son cycle de vie sont à concevoir avant exécution. Le projet reste incomplet.
