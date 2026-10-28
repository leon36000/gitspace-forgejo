# Historique des essais — FORGEJO-AGENT-STDIO-MCP-MERGE-001

## Essai 001 — refus avant fusion

Le premier essai a reçu HTTP 404 sur `GET /repos/forge-admin/mission-012-clamp/branches/main`. La PR appartenait au dépôt `forge-admin/agent-forge-core`; l’ancien helper de barrière conservait par défaut le chemin d’une fixture différente. La validation fail-closed a interrompu le run avant l’appel au gate; aucune fusion n’a été tentée. Le message est conservé sous `iteration-001-fail-closed.log`, SHA-256 `3d9041919baa35f010ec4d56c6142c47716cafa81fe4af23b91b04c21c7d5898`.

## Essai 002 — cible explicite, réussite

Le helper accepte maintenant l’identité de dépôt et de PR, ainsi qu’une lecture de branche cible fournie par le runner hôte. Commande exacte et résultat : `provenance.json`, `composition-result.json`. Code de sortie 0; gate appelé une fois, HTTP 200; readback et vérification post-fusion concordants. Le runner a arrêté l’instance locale normalement.

Aucun `verifier.key`, token, base Forgejo, configuration contenant credential ou transcript de session Claude n’a été copié dans ce dossier.
