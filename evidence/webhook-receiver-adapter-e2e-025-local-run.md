# WEBHOOK-025 — journal du récepteur vers l’adaptateur HTTP

Date : 2026-10-22. Worktree : `local/WEBHOOK-005-canonical-links`. HEAD examiné : `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`.

## Comportement observé

Le test `receiver_process_journal_reconciles_through_local_http_adapter` dans `src/bin/forgejo-reconcile-local.rs` lance le binaire réel `forgejo-webhook-local`, envoie deux webhooks HMAC synthétiques signés vers un port loopback éphémère, puis renvoie le même delivery ID. Les deux IDs distincts sont acceptés avec `duplicate=false` et les séquences 1 et 2; le retry est identifié comme doublon. Un corps différent, signé sous le même ID, est refusé avec `409 delivery_conflict`, `retryable=false`. Après arrêt SIGTERM, le journal contient exactement deux événements. Le processus redémarre sur ce journal; le retry d’origine reste reconnu comme doublon et le journal reste à deux événements. L’allocation du port loopback retente jusqu’à cinq fois pour réduire la course entre la sonde éphémère et le bind du processus.

Le test passe ces événements à `reconcile_inbox_events` et au client HTTP `LocalLoopbackForgejoApi` existant. Celui-ci appelle `curl` vers un second serveur loopback éphémère, avec un jeton synthétique fourni seulement par le fixture de test. Le payload webhook annonce `closed`; le snapshot HTTP courant annonce `open`. Les deux résultats rendent l’état `open`, un seul GET est effectué pour la ressource commune, et la requête contient le chemin et l’en-tête d’autorisation attendus.

Cette tranche modifie seulement les tests et leurs helpers dans le fichier du binaire d’adaptation; elle ne change pas le comportement de production. Les fixtures, le secret, le jeton et les données sont synthétiques. L’environnement est nettoyé pour le processus récepteur, qui est un vrai processus du dépôt; l’adaptateur HTTP s’exécute dans le harnais du test. Ce n’est donc pas un deuxième processus d’adaptation ni une instance Forgejo réelle.

## Vérifications

- Un répertoire Cargo neuf a été utilisé : `CARGO_TARGET_DIR=/tmp/webhook-receiver-adapter-025-clean-target cargo build --offline --locked --bin forgejo-webhook-local` réussit. C’est une précondition du test de processus, qui localise explicitement le binaire frère dans `target/debug` ou `target/release`.
- Depuis cette cible neuve, `CARGO_TARGET_DIR=/tmp/webhook-receiver-adapter-025-clean-target cargo test --offline --locked --bin forgejo-reconcile-local receiver_process_journal_reconciles_through_local_http_adapter -- --nocapture` passe 1/1; journal `webhook-receiver-adapter-e2e-025-clean-focused.log`.
- Depuis la même cible, `CARGO_TARGET_DIR=/tmp/webhook-receiver-adapter-025-clean-target cargo test --offline --locked --all-targets` passe 23 suites, 210 tests réussis, 0 échec et 2 helpers subprocess ignorés; journal `webhook-receiver-adapter-e2e-025-all-targets.log`.
- `CARGO_TARGET_DIR=/tmp/webhook-receiver-adapter-025-clean-target cargo clippy --offline --locked --all-targets -- -D warnings -A clippy::too_many_arguments -A clippy::bool_assert_comparison -A clippy::manual_div_ceil` : réussi avec trois catégories de lints préexistantes neutralisées; journal `webhook-receiver-adapter-e2e-025-clippy.log`.
- Après revue de code indépendante, les assertions `duplicate=false` des deux append neufs, la précondition de compilation du binaire, le démarrage avec cinq ports tentés au plus et le libellé « fixture HTTP » ont été corrigés/documentés puis revérifiés. `cargo fmt --all -- --check` et `git diff --check` réussissent; journaux `webhook-receiver-adapter-e2e-025-rustfmt.log` et `webhook-receiver-adapter-e2e-025-diff-check.log`.

## Limites et suite

Ce test vérifie la composition locale entre le processus récepteur, son journal, la reprise après arrêt et le client HTTP réel du binaire. La réponse API demeure une fixture contrôlée; aucun serveur Forgejo, credential réel, webhook envoyé par Forgejo, PR, vérificateur, fusion ou LLM n’est exercé. Le test ne démontre ni sécurité de déploiement, ni valeur comparative, ni réduction de coût ou de délai. Le binaire de réconciliation normal reste épinglé à `http://127.0.0.1:3130`; le test utilise une configuration et un credential provider synthétiques dans le harnais, sans assouplir cette limite de production.

Suite utile : décider si l’adaptateur doit devenir une interface de bibliothèque testable entre processus, ou garder cette couture de test au niveau du binaire jusqu’à un contrat de service explicite. Avant toute instance Forgejo réelle, il reste à démontrer l’origine du webhook, l’authentification du readback, la politique d’autorisation et le lien vérifiable entre la base relue et la PR candidate.
