# WEBHOOK-REVIEW-SUBTYPE-BINDING-001 — contrat réel des événements Forgejo

Date : 25 octobre 2026. Produit dans le worktree local sale `local/WEBHOOK-005-canonical-links`, basé sur HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`; les fichiers courants ne sont pas attribués à ce commit propre.

## Source primaire et défaut reproduit

Forgejo officiel v17.0.0 au commit `0d791421f85ca13a63b665053d81ea765fbbc571` déclare dans `modules/webhook/type.go` les noms wire `pull_request_review_approved`, `pull_request_review_rejected`, `pull_request_review_comment` et `pull_request_comment`. `services/webhook/notifier.go::PullRequestReview` choisit les trois noms de revue et sérialise le même nom dans `review.type`. `CreateIssueComment` émet `pull_request_comment` avec les objets `pull_request` et `comment`. Ces deux fichiers sont inchangés dans le scratch : blob `modules/webhook/type.go` `ecb75083f54cbdf110a76a826fb59349cfd0716a`, SHA-256 `ca6a94e1574be77bc4f5da1ee01a3e59147a148342b958577d12544dbd6d820c`; blob `services/webhook/notifier.go` `5aea521a0f2cb5d7513c98e9805d10e6a6b2901b`, SHA-256 `a67cca36f92104efe0feabf0c62e668e7543b979e5a7440e713ce93dd490004f`. Le scratch Forgejo est modifié pour d’autres POC; aucun code n’en est copié.

Le test rouge du vrai processus `forgejo-webhook-local` a montré que `pull_request_review_approved` recevait HTTP 422 `unsupported_event`, car l’application ne connaissait que son alias interne `pull_request_approved`. Un second test rouge a montré que le vrai événement wire `pull_request_comment` recevait HTTP 400 `event_payload_invalid`, parce que ce nom était interprété exclusivement comme l’alias d’un commentaire de revue qui exige `review.type`. Observations et commandes : `official-header-red-observation.txt` et `official-comment-red-observation.txt`.

Cette distinction corrige aussi l’interprétation de MISSION-012-COMPOSED-008/run 034 : son résumé consigne l’événement interne `pull_request_approved` et `review_type=pull_request_review_approved`, mais aucun en-tête HTTP brut. Le résumé ne prouve donc pas à lui seul que le nom wire Forgejo avait été accepté. La source primaire et le test rouge ci-dessus établissent que le récepteur antérieur refusait ce nom. Les étapes PR/merge de ce run ne sont pas rejouées ici.

## Changement

Le récepteur accepte maintenant les trois noms wire Forgejo et conserve la valeur reçue dans le type du journal. Il maintient les alias internes antérieurs pour compatibilité. Pour `pull_request_comment`, le corps signé désambiguïse l’événement : un objet `comment` désigne le commentaire PR wire; un `review.type=pull_request_review_comment` désigne l’ancien alias de commentaire de revue. Les approbations/rejets/commentaires de revue sont liés à `review.type` à l’ingestion et au replay; leur nom d’en-tête demeure non signé. Le driver de lecture accepte les valeurs wire et produit un alias interne seulement pour la corrélation des tâches.

## Vérifications

Commandes ciblées : `cargo test --offline --locked --test webhook_receiver_process_e2e -- --test-threads=1` → `official-header-process.log`; `cargo test --offline --locked --test forgejo_reconcile_capabilities_e2e actual_cli_discloses_versioned_local_test_capabilities -- --exact --nocapture --test-threads=1` → `official-header-capability.log`; `cargo test --offline --locked --test webhook_handler_contract -- --test-threads=1` → `official-header-ingest-contract.log`; `cargo test --offline --locked --test webhook_reconcile_contract -- --test-threads=1` → `official-header-reconcile-contract.log`; `python3 -m unittest tests.test_forgejo_emitted_webhook_e2e -v` → `official-header-python-contract.log`.

- La première exécution de l’acceptation des noms wire a échoué comme attendu : HTTP 422 `unsupported_event` avant la correction; l’observation est préservée.
- Après correction, `official-header-process.log` rapporte 8 tests de processus réussis, 0 échec et 21 ignorés. Il accepte les trois types wire de revue, les garde dans les enregistrements du journal, accepte le commentaire PR wire et vérifie les refus des sous-types contradictoires (alias historiques et noms wire officiels) ainsi que de l’en-tête générique.
- Le test de processus séparé accepte l’événement wire `pull_request_comment` portant `action=created`, `pull_request` et un objet `comment`; son résultat rouge 400 est conservé.
- `forgejo_reconcile_capabilities_e2e`: 1 test vert; découverte annonce les huit noms acceptés (wire et alias). `webhook_handler_contract`: 7 tests verts, incluant ingestion/relecture sous l’en-tête officiel. `webhook_reconcile_contract`: 23 tests verts; le replay réconcilie l’événement officiel.
- `tests.test_forgejo_emitted_webhook_e2e`: 24 tests Python verts; le parseur accepte les noms wire, normalise uniquement sa sortie interne et différencie le commentaire PR d’un commentaire de revue.
- Suite Rust complète : `cargo test --offline --locked --all-targets -- --test-threads=1` — 467 réussis, 0 échec et 33 ignorés en 34 groupes; journal `all-targets-official-wire-final.log`.
- Rustfmt, `git diff --check`, contrôle d’espaces des sources non suivies, compilation Python et garde de licence Cargo passent; cette dernière passe 7/7. Les commandes et résultats sont dans `format-and-diff-checks.txt` et `../cargo-license-state-explicit-001/verification-checks.txt`.

## Limites

Le récepteur traite des requêtes signées contre un processus local et consulte du code source Forgejo; aucune nouvelle livraison depuis une instance Forgejo vivante n’a été observée. L’en-tête événement n’est pas inclus dans le HMAC Forgejo. Lier ses valeurs aux sous-types du corps prévient certaines substitutions, mais ne rend pas l’en-tête authentique. Un `pull_request` générique sans discriminateur d’avis reste indiscernable par cette règle. Un rejeu du même événement sous un nouvel ID de livraison n’est pas réglé ici. Aucun modèle/LLM, AgentOS, RAG ou mémoire n’a été invoqué; aucun gain comparatif n’est établi.

Le manifeste local `SHA256SUMS` couvre 21 artefacts de preuve et s’exclut lui-même; le contrôle 21/21 est consigné dans `manifest-check.txt`.
