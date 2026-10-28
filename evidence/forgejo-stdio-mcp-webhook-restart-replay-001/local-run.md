# FORGEJO-STDIO-MCP-WEBHOOK-RESTART-REPLAY-001 — rejeu natif après redémarrage

Date : 26 octobre 2026. Le run utilise une instance Forgejo temporaire et un dossier d’état neuf sous `/tmp`. Il étend le parcours CONTINUATION-060 uniquement par l’arrêt/redémarrage du processus receiver entre l’acceptation de l’événement natif et le rejeu. Aucun service externe ni donnée privée n’a été utilisé.

## Parcours observé

Commande exacte et code de sortie : [command.txt](command.txt). Forgejo annonce `17.0.0+gitea-1.22.0`; SHA-256 du binaire `e5ec437cd6357e8d1b4580cb47ab1daa58a32b33971a63db63446a535d33adb7`. Le commit source exact ayant produit ce binaire n’est pas établi. Les hashes des outils/binaire Rust et leurs versions sont dans [trusted-build-provenance.json](trusted-build-provenance.json) et [source-hashes.json](source-hashes.json).

Le processus du gate crée la PR #2 du dépôt local, observe un échec d’acceptation puis une correction et un résultat indépendant 9/9. La PR est fusionnée HTTP 200 au commit `9bc4d0f0eb51d16553c17e7587689695145b069b`; l’issue est fermée. Forgejo envoie le vrai `pull_request/closed`, livré avec HTTP 202. Le journal du receiver contient alors quatre enregistrements, dont la livraison séquence 4, UUID `6320ce74-4a6d-4a4e-b420-9abe2c3795ea`.

Le premier receiver reçoit SIGTERM et sort avec le code 0. Un second processus reçoit le même bind loopback, le même chemin de secret local et le même journal. Le SHA-256 du journal avant/après redémarrage est identique (`9c3ffa9887c80633e286fb40ca4873b477052977f492342f0e1811923f335434`); il reste quatre entrées et l’UUID natif est présent une fois. Le pilote rejoue ensuite en mémoire le payload accepté, re-signé avec le secret de fixture. Le nouveau processus répond HTTP 202, `duplicate=true`, `reconcile_required=true`, même UUID et même séquence 4, sans changement des octets ni de la taille du journal. Résumé expurgé : [receiver restart](webhook-receiver-restart-summary.json) et [rejeu](webhook-duplicate-replay-summary.json).

La réconciliation qui suit lit l’API courante Forgejo : PR #2 fermée/fusionnée, head `88350d6f323139cfb3e9d0252a6f35f3e2d64ff9`, merge SHA/base `9bc4d0f0eb51d16553c17e7587689695145b069b`, verdict `current_state_read`, une lecture upstream. Les readbacks de livraison, tâche et réconciliation sont préservés dans `webhook-merged-*.json`. Le hook est désactivé puis relu.

## Vérifications

- E2E Forgejo local : exit 0, merge HTTP 200, acceptation initiale 5 échecs/4 réussites, correction 9/9, readback post-fusion réussi.
- `pytest -q tests/test_forgejo_live_pr_merge_e2e.py tests/test_forgejo_emitted_webhook_e2e.py` : 96 réussis.
- Suite active `pytest -q scripts tests` : 259 réussis et 60 sous-tests, zéro échec.
- `py_compile` des deux fichiers Python touchés : succès.
- `python3 scripts/verify_cargo_license_state.py` : `license=null`, `license_file=null`, `publish=[]`, `no_license_selected`, droits `not_assessed`, `NOASSERTION` personnalisé.

## Limites

Le doublon est injecté par le pilote local après redémarrage; ce n’est pas une seconde tentative émise par Forgejo et ne prouve pas la politique native de retry. Le redémarrage est gracieux par SIGTERM, pas une panne brutale/power-loss. Il vérifie la récupération du journal et de la déduplication par le vrai binaire local, puis la réconciliation canonique; il ne prouve pas une durabilité sur tous systèmes de fichiers, hôtes, ou corruption partielle.

L’agent reste `local-test-adapter` déterministe, sans LLM/AgentOS/RAG/mémoire; n=1, aucun benchmark ou gain. Le merger est l’admin de fixture (`--non-admin-merger` absent). Writers externes au verrou coopératif et CAS Forgejo universel ne sont pas couverts. Le state-dir complet sous `/tmp/forgejo-mcp-webhook-restart-replay-002` est conservé pour inspection locale; il n’est pas publié et aucun token, secret ou clé de vérificateur n’est copié dans ce dossier de preuve.
