# FORGEJO-STDIO-MCP-WEBHOOK-IDEMPOTENT-REPLAY-001 — run local

Date locale : 2026-10-26. Résultat : **réussi sur Forgejo local jetable**.

## État de licence Cargo relu

Le manifeste courant `agent-forge-core 0.1.0` conserve `publish = false`, sans champ Cargo `license` ou `license-file`. La relecture hors ligne retourne `license=null`, `license_file=null`, `publish=[]`, `decision=no_license_selected`, `redistribution_rights=not_assessed`, marqueur d’export `NOASSERTION`. Le garde courant passe 9/9. Aucun manifeste, fichier de licence ou droit de redistribution n’est ajouté ou choisi dans ce lot; les sorties et hashes sont conservés ici.

## Parcours et rejeu

Commande du run :

```sh
python3 scripts/run_forgejo_live_pr_merge_e2e.py \
  --forgejo-binary /tmp/forgejo-17.0.0 \
  --state-dir /tmp/forgejo-mcp-webhook-replay-001 \
  --non-admin-merger \
  --stdio-mcp-egress-during-merge \
  --observe-pr-webhook \
  --replay-pr-webhook
```

Forgejo annonce `17.0.0+gitea-1.22.0`; SHA-256 du binaire : `e5ec437cd6357e8d1b4580cb47ab1daa58a32b33971a63db63446a535d33adb7`. Rust/Cargo : 1.95.0; Python 3.12.3; Bubblewrap 0.9.0; système Linux aarch64. La provenance exacte du build Forgejo vers le commit source audité n’est pas établie. Aucun code Forgejo n’est copié.

Le processus `--stdio-mcp` `local-test-adapter` a d’abord échoué aux critères d’acceptation, a corrigé le candidat et obtenu 9/9. Les octets MCP, tête de PR, source lue après fusion et source utilisée pour le test post-merge ont tous le SHA-256 `78cc4a0a2cafa1d15b62adc8782af05445e1708268d618bf970de3b2340d407a`. La PR #2 est fusionnée en HTTP 200 au commit `67c96ab3fa8d21be6e3c2f6c1e66f6a6387a7a84`; le readback `main` et les 9 tests d’acceptation post-merge correspondent.

Forgejo a émis puis livré le vrai événement natif `pull_request:closed` : la tâche d’origine réussit HTTP 202. Le runner relit l’entrée acceptée dans son journal et, **dans le pilote local**, la re-sérialise et la re-signe en mémoire avec le secret local afin de rejouer le même UUID et le même payload JSON au receiver réel. Ce rejeu contrôlé n’est pas une seconde tentative envoyée par Forgejo. Le receiver répond HTTP 202, `duplicate=true`, `reconcile_required=true`, avec la séquence originale 4. Le journal demeure identique octet pour octet et reste à quatre entrées. Le hash du payload rejoué est `440f1d7ac4127472abd88b9e863d3432e2b472115d44bae81b2b6825c1eea81b`. Aucun corps, secret ou signature n’est conservé.

Après le rejeu, le CLI de confiance transmet son credential en pipe privé et relit l’état canonique auprès de Forgejo : `current_state_read`, même PR, tête `ba5e714f605fcc908397760d6d2a1a2ab96568f7` et même commit de merge `67c96ab3fa8d21be6e3c2f6c1e66f6a6387a7a84`. Le hook est désactivé puis relu. Les métadonnées Forgejo montrent une seule tâche native `closed` réussie; le rejeu de test s’adresse directement au receiver et n’ajoute pas de seconde tâche Forgejo.

Pendant la barrière, l’enfant MCP est vivant; les sondes API, Web et `git-receive-pack` sont bloquées, aucun credential n’est visible/hérité/persisté par l’enfant et le gate lit puis fusionne. Le runner refuse toujours le faux statut, le candidat périmé et les acteurs sans approbation/whitelist. La séparation se limite à Bubblewrap et aux routes observées sur cet hôte.

## Vérifications

`python-targeted.log` : 93/93 tests du runner et des événements webhook Forgejo émis. La suite des répertoires actifs `scripts tests` passe 256 tests et 60 subtests (`python-project-suite.log`). Une tentative de collecte depuis la racine (`python-suite.log`) s’arrête avant l’exécution après huit erreurs de collecte : des snapshots archivés sous `evidence/` dupliquent les noms des modules actifs et l’un d’eux référence un module de fixture absent. Ce n’est pas un résultat de test produit; la collecte bornée `scripts tests` est le résultat du projet. `cargo-license-tests.log` : 9/9 cas de refus et d’acceptation du garde; `license-metadata-readback.json` est la sortie réelle du script courant. `py_compile` du runner et de son test passe. Ce lot n’a modifié aucun code Rust; le full Cargo n’a pas été relancé. `git diff --check` est relancé à la clôture du lot.

## Limites

Le rejeu est une simulation locale contrôlée du transport d’un doublon, re-signée par le pilote qui détient le secret de test; il démontre le contrat du receiver et la réconciliation qui suit, pas le calendrier de retry Forgejo ni un retry réseau interrompu. `local-test-adapter`, n=1, sans LLM, AgentOS, RAG, mémoire ou benchmark; aucun gain général. Les writers Forgejo externes et CAS universel ne sont pas couverts. Le canari de conflit sémantique, les droits/licences des sources tierces et la comparaison appariée restent ouverts.
