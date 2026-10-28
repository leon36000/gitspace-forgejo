# WEBHOOK-MISSION-ADMISSION-002 — ingestion à admission, fixture locale

Date : 2026-10-22. Worktree isolé `local/WEBHOOK-005-canonical-links`, HEAD de départ `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`; `origin/main` local observé `6030e13e3887926f8629cbb22f785d5b4bab580a` (sept commits plus avancé). Aucun rebase, commit ou push.

## Changement

`tests/webhook_mission_admission_contract.rs:414` ajoute un contrat intégré qui appelle le parseur HTTP, le handler HMAC, l’inbox append-only, le replay vérifié, la réconciliation puis l’admission liée par l’hôte :

1. Une requête HTTP signée portant une notification obsolète `closed` est acceptée (202); la même livraison est dédupliquée sans deuxième append.
2. Le replay fournit l’événement journalisé. La lecture d’état Forgejo fixture `closed` refuse l’admission avec `IssueNotOpen`; le journal des missions reste vide.
3. Une nouvelle lecture fixture `open`, avec le binding hôte `REQ-001` révision 1 et le hash connu du fixture, admet la mission et réserve une génération.
4. Le test livre la même notification avec un nouvel UUID, ferme les handles puis rouvre les journaux. Le replay retrouve le même `mission_id`, la même clé d’idempotence et la même génération; le claim retourne `AlreadyClaimed` sans ajouter d’événement au journal mission.

Le test consomme donc les seams existants sans introduire un autre orchestrateur ni copier l’état canonique. Il s’agit d’un faux Forgejo et d’une clé HMAC de fixture en mémoire : aucun appel API, aucune donnée privée, aucun secret utilisateur, aucun AgentOS, Forge Code, RAG, mémoire, LLM, PR ou merge.

`src/bin/forgejo-reconcile-local.rs:1412` élargit la seule borne du test d’annulation de 150 ms à 1 s et nomme l’objectif précisément : l’enfant doit être annulé avant sa fin naturelle. La suite parallèle avait échoué sur l’ancienne assertion pour `/usr/bin/sleep 2` après l’échéance de 50 ms; le test ciblé repassait seul. Le superviseur sonde toutes les 1 ms et l’ordonnanceur peut ajouter du retard. La nouvelle borne est volontairement tolérante et prouve seulement un retour avant les 2 s naturels de l’enfant, pas une latence proche de l’échéance. Le code de production n’a pas changé dans ce correctif.

La revue read-only du test a recommandé ces vérifications supplémentaires d’identité, génération et absence d’append; elles ont été ajoutées avant le run final. La revue n’a exécuté aucun test, service ou API.

## Vérifications

Versions : `rustc 1.95.0 (59807616e 2026-04-14)`, `cargo 1.95.0 (f2d3ce0bd 2026-03-21)`. Toutes les commandes Cargo ci-dessous utilisent `--locked --offline`.

- `cargo test --locked --offline --test webhook_mission_admission_contract signed_http_delivery_replays_reconciles_then_admits_and_resumes_idempotently` : 1 réussi.
- `cargo test --locked --offline --all-targets` : 201 réussis, 0 échec, 2 helpers ignorés; sortie complète dans [webhook-mission-admission-002-all-targets.log](webhook-mission-admission-002-all-targets.log).
- `cargo build --locked --offline --release --all-targets` : réussite; sortie dans [webhook-mission-admission-002-release-build.log](webhook-mission-admission-002-release-build.log).
- `cargo fmt --all -- --check` et `git diff --check` : réussite.
- `cargo clippy --locked --offline --all-targets -- -A clippy::suspicious_open_options -A clippy::bool_assert_comparison` : code de sortie 0. D’autres avertissements préexistants demeurent (`too_many_arguments`, `manual_div_ceil`); le fichier est [webhook-mission-admission-002-clippy.log](webhook-mission-admission-002-clippy.log). Cette commande ne prouve pas un Clippy strict sans avertissement.

Empreintes du contenu final du lot :

- `tests/webhook_mission_admission_contract.rs` : `170d0913314edbcc8d24a6f8d59774676be49cfb9337d613343fdc6b7fa35dea`
- `src/bin/forgejo-reconcile-local.rs` : `2a0009398349c19b389c1699f1053a66a3d27044767183e8a15ac2d437511262`
- journal all-targets : `91239846d5209628819ecd47bca65b1198668660ba12427e6f5114b4a07c2923`
- journal release : `86e4461363b85d3db1ca1ef53bf56610d0df4a5dd8e05f39cdd9ef09bd20f0d1`
- journal Clippy : `a50ab88f7f710c545754c9b246a93d06e5040b53ba0625f9054d522e18abb018`

## Portée et limites

Le comportement intégré est testé jusqu’à la réservation de mission contre des fixtures en mémoire. Les journaux sont fermés et rouverts dans le même processus; aucun redémarrage de processus n’est prouvé. La source de l’issue, la politique de chemins protégés, les exigences et le dépôt ne sont pas lus depuis l’instance Forgejo; la construction d’un workspace et la modification réelle du code ne font pas partie de ce test. Les tests séparés antérieurs de `webhook_receiver_process_e2e` et de MISSION-012 ont une portée propre décrite dans leurs rapports; ce rapport ne les agrège pas en un parcours live unique.

L’adaptateur Forgejo du worktree est encore anonyme, épinglé à `127.0.0.1:3130` et au dépôt synthétique. Le readback local privé `127.0.0.1:3132` donnait 403 sans authentification; aucun contournement ni credential n’a été tenté. La prochaine étape d’exécution live dépend d’une source d’authentification hôte approuvée et d’une liaison protégée issue→exigence lisible par cette source. Aucun résultat de valeur comparative n’est mesuré.
