# Revue indépendante — MERGE-003, dérive exigences/politique

Date : 2026-10-22. Type : revue statique en lecture seule; aucun test lancé et aucun fichier modifié par le réviseur.

Périmètre : `tests/forgejo_preflight_process_e2e.rs` et `scripts/run_forgejo_preflight_cli_e2e.py` après ajout des cas de dérive des hashes d’exigences et de politique.

Constats vérifiés : le reçu conserve ses anciennes empreintes (`a…` et `b…`); chaque scénario change séparément l’attente fournie au CLI et attend `stale_requirements` ou `stale_policy`. Le runner lance les deux tests séparément sous `bwrap --unshare-net`. Les assertions et le drainage de la fixture contrôlent la requête GET initiale et les requêtes supplémentaires afin qu’une requête HTTP distincte ne soit pas masquée.

Résultat : aucun défaut actionnable signalé. Cette revue statique ne démontre pas que le binding attendu provient d’une source protégée; cette limite est conservée dans le rapport MERGE-003.
