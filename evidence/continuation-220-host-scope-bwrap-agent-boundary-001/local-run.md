# CONTINUATION-220 — enfant Bubblewrap dans un scope hôte attesté

Exécution locale du 27 octobre 2026, sur le worktree fortement modifié à HEAD de base `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. Environnement observé : Linux aarch64, Python 3.12.3, Bubblewrap 0.9.0, systemd 255. Aucun service externe, modèle, coût fournisseur, code Forgejo upstream ou état de production.

## Changement et question testée

`scripts/systemd_process_scope.py::launch_scoped_process` rend maintenant `close_fds=True` explicite au `Popen` qui lance `systemd-run`. Les seuls descripteurs transmis à cet exécutable sont donc les canaux standards déclarés par l’appel; aucun FD de capacité du superviseur n’est transmis implicitement. La sonde `scope_bwrap_probe.py` prépare et atteste le scope depuis l’identité hôte, puis lance l’enfant Bubblewrap dans ce scope, au lieu de demander à un processus déjà dans un user namespace d’accéder au bus systemd.

Le test démarre une fixture HTTP exclusivement sur `127.0.0.1` et un pipe/fichier contenant un marqueur synthétique qui n’est pas un credential Forgejo. Il rend le FD du pipe héritage-possible pour éprouver explicitement la fermeture des FD, puis démarre Bubblewrap avec des namespaces réseau et PID privés, `/tmp` et `/home` en tmpfs et des limites via `prlimit`. L’enfant essaie les routes API, Web et `git-receive-pack` de la fixture, recherche le FD canari et tente de lire le fichier canari.

## Résultat observé

Le processus hôte, pendant que l’enfant est vivant, demande `GET /api/v1/repos/forge-admin/agent-forge-core/pulls/1` à la fixture et reçoit HTTP 200 avec `ok`. L’enfant tente ces trois requêtes :

- `GET /api/v1/repos/forge-admin/agent-forge-core/pulls/1`
- `GET /forge-admin/agent-forge-core/pulls/1`
- `POST /forge-admin/agent-forge-core.git/git-receive-pack`

Chaque tentative échoue avant HTTP avec `ConnectionRefusedError` / errno 111. La fixture n’observe que le GET hôte attendu. Les namespaces réseau et PID de l’enfant diffèrent de ceux de l’hôte; son `/proc/self/cgroup` correspond au cgroup attesté avant lancement. L’enfant ne voit ni le FD hérité-possible ni le canari d’environnement ou de fichier; après sa sortie, le processus hôte lit toujours exactement le contenu du pipe synthétique. Le processus termine code 0 et le scope se nettoie avec verdict `clean`. Les observations structurées sont dans `scope-probe.log`.

Le module de tests existant `test_mission_012_live_composed_e2e.py` passe 14/14 après ce changement; `py_compile` et `git diff --check` passent. Le garde Cargo confirme toujours `license=null`, `license_file=null`, `publish=[]`, `no_license_selected`, `NOASSERTION` et droits `not_assessed`. Les sources, commandes et versions sont liées dans les fichiers adjacents.

## Portée et limites

C’est un test processus local `n=1` avec un enfant de sonde Python déterministe et un serveur HTTP fixture, pas un agent LLM, MCP host, Forgejo live, webhook ou gate Rust. Il prouve ce montage précis de scope hôte + Bubblewrap, la fermeture de ce FD synthétique, la séparation de ces namespaces et l’accès au listener loopback observé. Il ne prouve pas que le composeur MISSION-012 courant utilise déjà ce montage, que toutes les routes ou protocoles sont bloqués, ni qu’un agent réel ne peut pas exploiter un autre descripteur/canal, le même UID hôte, un privilège ou une évasion. La fixture gate ne constitue ni une décision de merge ni une preuve d’autorité CAS serveur.

Suite : intégrer le scope hôte au chemin existant de mission/MCP/Forgejo local avec `local-test-adapter`, garder le MCP et le gate côté host sur des interfaces étroites, puis vérifier dans un même run webhook → candidat → PR → gate → merge contrôlé et post-readback avec les canaris actifs, reprise/annulation et limites ressources. Aucun appel LLM payant n’est autorisé par ce lot. Le projet reste incomplet.
