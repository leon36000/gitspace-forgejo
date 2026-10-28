# Revue statique indépendante — ADMISSION-MCP-001

Date : 2026-10-23 UTC. Revue read-only ciblée de `src/mission_012_host.rs`, `src/bin/mission-012-mcp-host.rs` et `tests/mission_012_mcp_lifecycle.rs`. Aucun test, réseau ou fichier de code n’a été exécuté/modifié par cette revue.

## Résultat

Aucun nouveau défaut matériel trouvé dans le lien admission → claim durable → hôte MCP.

- Le test appelle `admit_and_claim_mission_v2`; requête, allocation, génération, commit et claim viennent de son reçu/journal. Le host reçoit cette requête et le même journal. La source est une fixture Git locale synthétique. La configuration hôte est encore construite manuellement par le test; le processus `forgejo-mission-local` ne participe pas.
- Le host appelle `bind_workspace_to_admitted_commit` avant `acquire_lease` et avant toute lecture de stdin. Il exige un commit SHA-1 complet connu dans le dépôt local de confiance et tête de la branche épinglée, compare le snapshot de base et le candidat initial au tree Git, vérifie l’exigence protégée, puis lie tree/branche/manifest au reçu. Les refus vérifient l’absence de lease, reçu et réponse MCP.

## Limite à conserver

La tête de branche n’est vérifiée qu’au démarrage du host. Le test périmé fait avancer la branche avant le démarrage; il ne couvre ni une avance après le binding pendant l’exécution, ni un contrôle au moment d’une fusion. Cette tranche n’a aucun chemin de fusion. Le binding du SHA du workspace ne constitue donc pas une preuve de fraîcheur au moment d’une éventuelle fusion; le vérificateur et l’autorité de fusion doivent relire la base courante et invalider la preuve si elle a changé.

Cette revue est statique; elle n’a pas reproduit les journaux ni évalué l’usage réel. Elle est faite par un sous-agent Codex, et **n’est pas** la revue Sonnet 5.5 demandée à la session Claude Code.
