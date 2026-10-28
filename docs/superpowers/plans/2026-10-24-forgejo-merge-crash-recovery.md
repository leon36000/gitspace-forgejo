# Reprise du merge Forgejo après arrêt — plan d’implémentation

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [x]`) syntax for tracking.

**Goal:** Après un arrêt autour du POST de merge, reprendre le même candidat par readback Forgejo sans répéter automatiquement une fusion ambiguë.

**Architecture:** Étendre le `MissionExecutionJournalV1` hash-chaîné avec un intent de merge fsync avant le POST et une résolution durable avant le retour de succès. Le premier passage peut effectuer un POST unique; les passages ultérieurs portant le même intent sont readback-only, et n’acceptent le succès que si l’état exact PR/base/candidat/parents/arbre/issue est confirmé.

**Tech Stack:** Rust 2024, `EventJournal` JSONL existant, binaire `forgejo-reconcile-local`, tests Rust de contrat et de processus, fixture HTTP loopback stateful; aucun nouveau crate ni service.

**Spec:** [Décision reprise du merge après interruption](../../architecture.md#reprise-du-merge-après-interruption-du-processus).

## Contraintes globales

- Garder les changements locaux, aucun déploiement, publication ou accès de production.
- Ne jamais laisser l’exécuteur modifier le journal de merge, le policy, la preuve ou les critères protégés.
- Persister l’intent hôte avant d’émettre le POST; journaliser le résultat avant de retourner le succès.
- Un intent existant n’autorise jamais un autre POST; son traitement reprend par readback seulement.
- Absence, indisponibilité, version ou binding incohérent donnent `inconclusive`, jamais `merged`.
- Ne pas prétendre à une garantie contre un writer Forgejo externe ou un processus du même UID.
- Ne pas modifier `license`/`license-file` ni la configuration de publication.

## Points de revue

- Crash après fsync de l’intent, avant POST : PR ouverte, résultat inconclusif, aucun POST de reprise.
- Crash après commit côté Forgejo, avant réponse/readback : un seul POST total; reprise readback-only vérifie le merge exact.
- Readback indisponible ou divergent (base, head, commit parents, tree, issue) : aucun succès et aucun POST de reprise.
- Crash après readback vérifié, avant résolution journalisée : la reprise relit Forgejo puis écrit la résolution avant de répondre.
- Deux reprises simultanées ou une admission/preuve/policy modifiée : sérialisation par mission/base et refus de bindings différents.

---

### Task 1 : transitions durables d’intent/résolution

**Fichiers :**
- Modifier : `src/lib.rs` — append/fsync et validation des événements versionnés du journal.
- Modifier : `src/agent_runtime.rs` — API qui vérifie la claim et l’intent sous verrou de mission.
- Test : `tests/mission_execution_journal_contract.rs`.

**Interfaces :** l’identité de mission vient de `BaseBranchScope`, issue, requirement, allocation/génération et `AgentExecutionRequestV1`. L’API rend un intent durable lié au scope, PR, base/head, requirement/policy/proof/review/admission hashes, puis exige une résolution autorisée sous le même verrou. Les résolutions legacy `inconclusive` ne ferment pas l’intent même si leur ancien champ `terminal` indique `true`; la reprise terminale s’ajoute avec une clé d’idempotence distincte.

- [x] Écrire les tests rouges `merge_attempt_intent_is_durable_before_effect`, `unresolved_merge_intent_is_reused_read_only`, `same_pr_with_changed_binding_is_rejected` et `resolved_merge_attempt_replays_saved_result`.
- [x] Exécuter `cargo test --offline --locked --test mission_execution_journal_contract` et constater les nouveaux cas en échec avant implémentation.
- [x] Ajouter les transitions append-only `mission_merge_attempt_started_v1` et `mission_merge_attempt_resolved_v1`, liées à l’allocation/génération et rejouées strictement sans accepter les événements incomplets ou contradictoires.
- [x] Ajouter une API de `MissionExecutionJournalV1` qui, sous verrou mission, valide la claim, fsync l’intent avant son callback réseau et empêche l’effet pour un intent existant; la résolution est appendue/fsync avant de rendre un résultat positif.
- [x] Rejouer les quatre tests et la suite du journal; vérifier que les anciennes lignes JSONL restent lisibles.

### Task 2 : gate merge avec mode neuf ou readback-only

**Fichiers :**
- Modifier : `src/bin/forgejo-reconcile-local.rs` — `merge_authority_decision`, `run_final_merge_window`, `post_merge_once_then_readback`, `finalize_merge_post_readback`.
- Test : tests unitaires dans le module binaire `forgejo-reconcile-local.rs`.

**Interfaces :** le gate fournit une struct versionnée et hôte `MergeAttemptBindingV1`, calculée après validation des entrées protégées et avant l’effet. Les callbacks distinguent strictement `NewAttempt` (un POST maximum) et `RecoverOnly` (readback GET seulement). Les octets d’autorisation hôte sont relus après les derniers GET Forgejo et immédiatement avant le POST.

- [x] Ajouter d’abord les tests rouges pour un intent retrouvé avec PR déjà fusionnée exactement, PR encore ouverte, et head/base/parents/tree/issue divergents.
- [x] Déplacer la reprise de l’intent avant la préflight qui exige un PR ouvert, après vérification de la claim, de l’admission, de la preuve et du binding des fichiers de policy/revue au hash de l’intent.
- [x] Scinder la préflight finale, le POST unique et le readback; écrire l’intent durable après les derniers contrôles et avant tout POST.
- [x] Pour un intent ancien, exécuter uniquement les readbacks authentifiés; enregistrer `merged` uniquement après correspondance complète, sinon `inconclusive` sans POST.
- [x] Ajouter un résultat de résolution idempotent et expurgé; ne stocker aucun token ni secret dans le journal.
- [x] Lancer les tests unitaires ciblés pour effets HTTP, lecture seule après intent, état périmé et résultat déjà résolu.

### Task 3 : test de vrai redémarrage de processus

**Fichiers :**
- Créer : `tests/forgejo_merge_post_crash_recovery_process_e2e.rs`.
- Modifier : `Cargo.toml` et le binaire seulement si un failpoint de test strictement compilé sous feature/debug assertion est nécessaire.
- Créer : `evidence/forgejo-merge-crash-recovery-001/`.
- Mettre à jour : `docs/architecture.md`, `docs/checkpoint.md`, `docs/implementation-plan.md`.

**Interfaces :** le test lance le binaire `CARGO_BIN_EXE_forgejo-reconcile-local` comme enfant avec `env_clear`, fixtures privées et credentials par pipe; un fixture HTTP loopback stateful garde l’état PR/main/issue entre processus. Ce fixture implémente le sous-ensemble Forgejo requis mais n’est pas le serveur Forgejo.

- [x] Ajouter un test de processus ignoré hors exécution réseau isolée, avec la fixture réservant uniquement `127.0.0.1:3132` dans `bwrap --unshare-net`; enregistrer les requêtes POST/GET.
- [x] Provoquer une vraie terminaison SIGKILL après que le serveur a fusionné et journalisé l’unique POST mais avant réponse/readback du premier processus; redémarrer le même CLI et prouver le succès par readback-only.
- [x] Provoquer l’arrêt après intent fsync mais avant POST; redémarrer sur PR ouverte et prouver `inconclusive`, zéro second POST.
- [x] Tester readback absent/divergent, duplicate retry et deux reprises concurrentes; vérifier un seul intent et au plus un POST.
- [x] Exécuter la suite de contrats, le test process isolé, `cargo test --offline --locked --all-targets`, Clippy avec allowances historiques, rustfmt, `git diff --check` et tous les manifestes SHA-256.
- [x] Documenter commandes, versions, hashes, effectifs et limites; ne pas appeler cela exactly-once ou protection contre les writers externes.
