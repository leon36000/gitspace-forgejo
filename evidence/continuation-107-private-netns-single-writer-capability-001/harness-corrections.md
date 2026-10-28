# Tentatives de préparation corrigées

Aucun des essais ci-dessous n’a lancé de serveur ni contacté Forgejo.

1. `bwrap --unshare-all --disable-userns ...` a été refusé au démarrage : `--disable-userns requires --unshare-user`. L’appel corrigé reprend l’ordre de `scripts/run_mission_012_live_agent_pr_e2e.py`.
2. Avec `--unshare-user`, le premier script a tenté `ip link set lo up` et reçu `RTNETLINK answers: Operation not permitted`. La lecture `ip -o link show lo` confirme ensuite que l’interface loopback était déjà `UP`; cette mutation inutile a été retirée. Aucun privilège réseau n’a été ajouté.
3. Le premier script de diagnostic du statut de l’interface a échoué avec `NameError: json is not defined`; c’était une erreur du harnais. Le diagnostic corrigé a terminé avec succès.
4. La première version du vérificateur de manifeste a appelé `sha_bytes` avant sa définition. Cette erreur de l’outil de vérification n’affecte pas la sonde; la fonction a été déplacée avant l’appel et le vérificateur est rejoué après le nouvel append des empreintes.

La sonde finale reproductible est `probe_private_loopback.py`; son résultat complet est dans `probe-result.json`, stderr est vide et le code de sortie est 0.
