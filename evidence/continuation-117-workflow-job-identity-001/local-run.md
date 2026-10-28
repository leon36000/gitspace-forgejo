# CONTINUATION-117 — lier le readback d’un job Actions à son index signé

Date : 26 octobre 2026. Worktree : `/home/pc6/Documents/Codex/2026-10-21-mission-construire-une-forge-con-ue/.runtime/webhook-005-workspace`; base HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. Le dépôt était déjà fortement modifié; les changements de ce lot ne sont pas commités ou stagés.

## Constat et changement

La revue read-only de `reconcile_workflow_job` a trouvé que le code liait le `job.id` et le `run_id`, puis vérifiait que l’URL de l’API était syntaxiquement sous le bon run. Il ne comparait pas le segment `/jobs/{index}` à celui de `job.html_url` dans le payload HMAC. Le test-first `workflow_job_readback_refuses_api_url_for_another_job_index` confirme le défaut avant correctif : le readback renvoie `Ok(ReconciledState)` avec `id=9001`, run `7001`, mais un lien `/jobs/0` lorsque l’événement signé pour ce job portait `/jobs/1` (`red-test.log`).

Le correctif extrait l’index décimal canonique de l’URL signée à partir du suffixe `/actions/runs/{index_in_repo}/jobs/{job_index}/attempt/{attempt}` et le conserve dans l’identité interne `ResourceId::WorkflowJob`. La validation du webhook refuse une URL absente ou mal formée avant append; les métadonnées de capacités indiquent désormais ce besoin. Au readback, l’URL courante de l’API doit être sous l’URL canonique du run parent et son index doit correspondre à celui de l’événement. La tentative API peut être plus récente. Aucune requête n’est construite à partir de l’URL du webhook; l’API fournit toujours le lien et l’état affichés.

La matrice de refus et l’architecture ont été mises à jour. Le schéma JSON de résultat CLI n’a pas changé; `job_index` reste une liaison interne et n’est pas exposé dans la projection actuelle.

## Vérifications

- Avant le correctif : le test contrat échoue exactement parce que la réponse API incohérente est acceptée.
- Après : `webhook_reconcile_contract` — 54 réussis, 0 échec.
- Après : `webhook_handler_contract` — 26 réussis, 0 échec, y compris le refus d’une URL d’événement mal formée avant append et les champs du contrat publié.
- Sous Bubblewrap avec espaces réseau/PID séparés, runner de processus réel receiver + CLI, fixture loopback : un événement signé index 1 et une API index 0 retournent `inconclusive/invalid_current_state`, champ `html_url`, sans `canonical_state` et sans modification du journal — 1/1.
- Régression après changement : le scénario processus existant du premier job index 0 avec tentative courante valide reste accepté — 1/1.
- `cargo test --offline --locked --all-targets --no-run` compile toutes les cibles de test.
- `cargo fmt --all -- --check` et `git diff --check` passent.

Les commandes, toolchain, sorties complètes et empreintes de sources sont dans les fichiers voisins. `SHA256SUMS` couvre le reçu.

## Portée et limites

Les tests de contrat et de processus utilisent des fixtures locales; aucune instance Forgejo live ou runner Actions n’a produit la paire incohérente. Il s’agit d’une défense contre une réponse API mal liée, pas d’un défaut observé en usage Forgejo. Le processus isolé testé lit l’inbox et une API loopback, mais le lot ne rejoue aucun parcours de PR/merge, n’utilise ni AgentOS ni LLM, ne mesure aucun coût/token et ne fournit aucun benchmark comparatif. L’isolation egress et le parcours supervisé précédents sont référencés dans les checkpoints existants; ils n’ont pas été rejoués ici. Aucun secret ou service externe n’a été utilisé.

La mission reste incomplète : comparaison contrôlée Forgejo + agent + CI, runtime LLM autorisé à coût mesurable, choix de licence produit/droits de redistribution et certaines limites de frontière Forgejo restent ouverts.
