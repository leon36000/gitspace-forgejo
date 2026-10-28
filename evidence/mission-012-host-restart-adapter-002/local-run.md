# MISSION-012-HOST-RESTART-ADAPTER-002

Date : 25 octobre 2026. HEAD observé : `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`, branche `local/WEBHOOK-005-canonical-links`. Worktree déjà fortement modifié; aucun commit, publication ou déploiement.

## Portée

Le test de processus `sigkill_before_and_after_workspace_effect_are_recovered_under_a_new_fence` lance le host avec la feature debug `test-crash-injection`. Pour chaque point de panne (`before_effect`, `after_effect`), il tue réellement le processus host par SIGKILL après l’intent durable, attend l’expiration du bail, renouvelle l’ID de lease, puis lance `scripts/mission_012_test_agent_adapter.py` comme client MCP externe du nouveau host. L’adaptateur lit le contexte, exige un checkpoint `unverified` lié au contenu et à son hash, observe de nouveau l’échec protégé, relit le code, rejoue exactement l’écriture par MCP et vérifie le succès protégé. Le test vérifie également un seul résultat de write au fencing token 2 et le reçu final `candidate_ready`.

L’adaptateur est un fixture déterministe local; il ne contacte pas de modèle, ne lit pas et n’écrit pas directement l’espace candidat. L’admission et Forgejo sont simulés/synthétiques dans ce test processus; aucune PR ou fusion n’est exécutée ici.

## Résultats frais

- `cargo test --offline --locked --features test-crash-injection --test mission_012_mcp_lifecycle sigkill_before_and_after_workspace_effect_are_recovered_under_a_new_fence -- --nocapture` : 1 test réussi, 0 échec, 26 filtrés; les deux points SIGKILL sont exercés. Sortie : `sigkill-adapter-e2e.log`.
- `cargo test --offline --locked --test mission_012_mcp_lifecycle -- --test-threads=1` : 25 réussis, 0 échec, 1 ignoré; 10,76 s. Sortie : `lifecycle-suite.log`.
- Clippy ciblé sur le test avec feature : réussite sous les six allowances historiques documentées dans `clippy.log`.
- `cargo fmt --all -- --check` et `git diff --check` : réussites; le second contrôle ne couvre que les chemins suivis, tandis que le test Rust ajouté est non suivi dans ce worktree et est couvert par rustfmt et sa compilation/exécution.

Les hashes des sources et la toolchain observée sont dans `source-sha256.txt` et `toolchain.txt`. Le manifeste local couvre les fichiers du présent dossier sauf lui-même.

## Limites

Ce lot relie la récupération du host à son vrai client fixture MCP, mais ne relie pas encore ce redémarrage à une PR Forgejo, à une vérification Forgejo indépendante ou à une fusion. Il n’exerce pas un agent LLM, une panne d’alimentation, un processus malveillant du même UID, une base avancée ou un environnement multi-tenant. La feature d’injection reste désactivée par défaut et le build release la refuse, vérification déjà archivée avec `MISSION-012-HOST-RESTART-RECOVERY-001`. Ces résultats ne démontrent aucun gain face à Forgejo + agent + CI conventionnels.

Suite utile : composer cette reprise avec le parcours PR/autorité de fusion sur un Forgejo jetable, sans rejouer les runs 046–050; conserver ensuite la comparaison contrôlée et le parcours LLM comme critères ouverts faute de modèle/coût autorisés.
