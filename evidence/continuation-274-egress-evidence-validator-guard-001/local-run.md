# CONTINUATION-274 — validation stricte des refus d’egress

Date : 2026-10-27  
Worktree : `.runtime/webhook-005-workspace`  
HEAD de base : `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0` (dépôt fortement modifié; aucun commit créé).

## Constat et changement

Le validateur `validate_live_egress_result` acceptait toute valeur entière de `sandbox_errno` et ne comparait pas les entrées du tableau `sandbox_connect_errors` aux observations route par route. Deux mutations adversariales ont été testées avant le changement : `EIO` (`errno=5`) était traité comme preuve de blocage réseau, et une contradiction entre le tableau redondant et la route était ignorée. Les deux nouveaux tests échouaient alors parce qu’aucun `ValueError` n’était produit; cette sortie rouge est résumée dans `red-test-observation.md`.

Le runner émet maintenant le schéma 2. Le validateur accepte seulement `ECONNREFUSED`, `ENETUNREACH` ou `EHOSTUNREACH`, puis compare le code errno, le type d’exception et le port des observations ordonnées avec chaque route. Il maintient la lecture des reçus historiques schéma 1, qui omettaient `sandbox_connect_errors`, mais applique aussi l’allowlist errno aux routes v1. Les reçus v2 exigent le tableau redondant complet et cohérent.

## Vérifications

- `python3 -m unittest -v tests.test_forgejo_agent_egress_live_e2e` : 12 réussis, dont les deux nouveaux refus adversariaux et l’acceptation d’un reçu v1 historique sans le tableau redondant.
- Relecture de `evidence/forgejo-agent-egress-live-001/result.json` par le validateur courant : acceptée comme schéma v1; SHA-256 `06951f8300670f88f8c2c4887f7dd3d8c8d168e5ebb32c8df2c6c779a5576c3a`.
- `python3 -m py_compile scripts/run_forgejo_agent_egress_live_e2e.py tests/test_forgejo_agent_egress_live_e2e.py` : réussie.
- `git diff --check` : réussie.

Les sorties gardées sont `validator-tests.log`, `historical-live-artifact-readback.json`, `pycompile.log` et `diff-check.log`. Commandes et versions : `commands.txt`, `toolchain.txt`. Hashes sources : `source-sha256.txt`.

## Limites

Il s’agit d’un contrôle de validation de preuves, pas d’une nouvelle sonde réseau. Aucun Forgejo n’a été démarré dans ce lot. Le reçu v1 accepté provient d’une exécution antérieure `n=1` avec `local-test-adapter`; aucun modèle ou AgentOS n’a été utilisé. La cohérence et les hashes locaux ne donnent pas au reçu une signature indépendante. Le checkpoint confirme que la politique du superviseur demeure non vérifiée indépendamment et que l’exclusion du credential de fusion dans la composition MISSION-012 courante n’est pas prouvée. Il reste à auditer ces frontières, puis à compléter les refus fonctionnels et le comparatif contrôlé.
