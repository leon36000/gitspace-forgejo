# CONTINUATION-012 — refuser `review` hors schéma dans un événement `issues`

Date locale : 25 octobre 2026. Dépôt produit local : `.runtime/webhook-005-workspace`, HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. Le worktree contenait déjà un grand nombre de modifications; le lot touche seulement le parseur webhook, ses tests, le checkpoint et le plan. Environnement détaillé dans `tool-versions.txt`.

## Source et lacune

Dans Forgejo officiel v17.0.0 au commit `0d791421f85ca13a63b665053d81ea765fbbc571`, `modules/structs/hook.go::IssuePayload` définit l’événement d’issue avec `action`, `number`, `changes`, `issue`, `repository`, `sender`, `commit_id` et `label`; aucun champ `review` n’est défini. Le blob est `9c7f1deed1c38eeb435ab976bb8429c5458c7f2b` et le fichier est inchangé dans le scratch. Une note de provenance figure dans `forgejo-issue-payload-source.txt`; aucun extrait de code Forgejo n’est copié dans le produit.

Dans `src/webhook/reconcile.rs::validate_event_payload_kind`, la condition historique rejetait un champ `review` seulement lorsqu’il était un objet. `null`, booléen, texte et tableau traversaient l’ingestion, pouvaient être ajoutés au journal, puis passaient par la réconciliation et un GET Forgejo. Les autres champs d’un type d’événement étranger (`pull_request`, `comment`) étaient rejetés quelle que soit leur valeur.

## Correction et résultats

La condition rejette désormais toute présence de `review` pour `issues`.

- **TDD rouge, réconciliation :** `cargo test --locked --offline --test webhook_reconcile_contract issue_notification_rejects_non_object_review_member_before_api_calls` échoue à cause d’un résultat `Ok(ReconciledState)` après le GET, au lieu de `InvalidEnvelope`.
- **TDD rouge, ingestion :** `cargo test --locked --offline --test webhook_contract signed_issue_payload_with_non_object_review_member_is_rejected_without_journal_side_effect` échoue, car le payload signé avec `review:null` est accepté.
- **Vert après correction :** ces deux commandes passent 1/1; les quatre valeurs null/bool/texte/tableau sont refusées. La réconciliation n’effectue aucun GET; l’ingestion n’ajoute aucune ligne au journal pour les quatre deliveries signées.
- Suite entière `cargo test --locked --offline --test webhook_reconcile_contract`: 37 réussis, 0 échec.
- Suite entière `cargo test --locked --offline --test webhook_contract`: 10 réussis, 0 échec.
- `cargo fmt --all -- --check` et `git diff --check`: réussis.
- `python3 scripts/test_cargo_license_state.py`: 9 réussis.
- `python3 scripts/verify_cargo_license_state.py`: `status=verified`, `license=null`, `license_file=null`, `publish=[]`, `license_decision=no_license_selected`, `redistribution_rights=not_assessed`, `spdx_package_license_declared=NOASSERTION`.

Les commandes et sorties complètes sont conservées dans les fichiers `.log` de ce dossier.

## Portée et limites

Cette tranche vérifie deux frontières locales : acceptation du webhook avant journalisation et relecture/réconciliation avant accès à l’API. Elle n’ajoute pas un nouveau serveur HTTP Forgejo ni un parcours API live; les précédents tests de processus et de Forgejo ne sont pas rejoués. Le contrat est corroboré par une struct Forgejo épinglée, pas par une nouvelle émission live.

Aucune licence n’est sélectionnée; aucune modification Cargo n’a été nécessaire, car le manifeste déjà désactive la publication et laisse les champs Cargo de licence vides. Aucun AgentOS, runtime LLM, coût, credential réel, production ou service externe n’est utilisé. Le projet reste incomplet, notamment pour l’autorité exclusive sur tous les writers Forgejo, le parcours modèle autorisé et la comparaison contrôlée.
