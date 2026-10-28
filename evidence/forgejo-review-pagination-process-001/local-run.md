# FORGEJO-REVIEW-PAGINATION-PROCESS-001 — incohérence paginée bloquée avant merge

Date : 25 octobre 2026. Worktree `local/WEBHOOK-005-canonical-links`, HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`; worktree dirty. Le runner consigne aussi le digest de l’arbre source compilé dans `isolated-e2e.log`.

## Scénario et résultat

Le test ignoré `inconsistent_review_totals_stop_merge_authority_process_before_post` lance le vrai binaire `forgejo-reconcile-local --merge-local-test` sous Bubblewrap avec réseau et PID isolés. Sa fixture loopback renvoie les deux pages complètes (50 puis 2 avis) mais change `X-Total-Count` de 51 à 52 entre elles.

La CLI termine en refus structuré : `verdict=inconclusive`, `error.code=review_pagination_inconsistent`, `stage=review-list-read`, `retryable=true`, avec action de jeter les pages et de relire toute la liste. Le test confirme que les deux pages ont été demandées et qu’aucune requête POST n’a été envoyée. Résultat processus ciblé : 1 réussi, 0 échec, 4 tests filtrés.

Cette preuve processus complète les tests présents dans `src/bin/forgejo-reconcile-local.rs` (`merge_authority_refuses_changed_review_total_before_any_merge_post` et `forgejo_review_page_adapter_reports_changed_total_as_retryable_inconclusive`); elle ne modifie pas la logique de production.

## Licence Cargo et runtime agent

Lecture fraîche par `scripts/verify_cargo_license_state.py` : `license=null`, `license_file=null`, `publish=[]`, `license_decision=no_license_selected`, `redistribution_rights=not_assessed`, `status=verified`. Les sept contrats du garde passent. Aucun choix de licence ni droit de redistribution n’est inféré.

Les recherches locales antérieures constatent `AgentRuntimeV1` et `FixtureRuntime` de test, mais aucun fournisseur de production ou interface AgentOS/Forge Code/RAG/mémoire autorisée. Ce test n’invoque pas de modèle ou service externe et n’engage aucun coût.

## Reproduction et limites

Les commandes exactes, sorties brutes, versions, empreintes de source et codes de sortie sont enregistrés dans ce dossier. Le run de processus est exécuté par `scripts/run_forgejo_mission_delivery_process_e2e.py --only ...`, qui compile hors sandbox puis lance le test dans un namespace réseau isolé.

Forgejo est une fixture HTTP déterministe sur le profil loopback compilé `127.0.0.1:3132`, pas une instance Forgejo live. Les pages reproduisent la réponse d’un adaptateur API, sans changement concurrent réel dans Forgejo. Le test ne démontre ni un parcours LLM, ni le bénéfice général de la couche agentique, ni la comparaison contrôlée avec une CI conventionnelle. Les autres refus et le parcours webhook→fusion sont couverts par leurs preuves distinctes; ils ne sont pas rejoués ici.
