# Historique des essais

Tous les essais utilisent une nouvelle instance locale Forgejo et un nouveau dossier /tmp; aucune instance existante n’a été réutilisée ou nettoyée.

| Run | Résultat | Diagnostic |
| --- | --- | --- |
| /tmp/forgejo-mcp-same-source-001 | Refus fail-closed au préflight final, code 1 | not_mergeable; aucune fusion déclarée. |
| /tmp/forgejo-mcp-same-source-002 | Même refus, code 1 | Snapshot après refus : tête et base attendues, mergeable=false, voir attempt-002-final-preflight-state.json. |
| /tmp/forgejo-mcp-same-source-003 | Réussite, code 0 | Poll explicite true → false → false → true → true → true après restauration, puis PR #2 vérifiée, revue, fusionnée et relue. Source identique à l’écriture MCP. |

Le premier refus a empêché toute fusion. Le second ajoutait un readback de diagnostic. Le troisième a changé seulement l’attente de mergeabilité du candidat PR restauré : il attend une preuve stable après le recalcul Forgejo asynchrone.
