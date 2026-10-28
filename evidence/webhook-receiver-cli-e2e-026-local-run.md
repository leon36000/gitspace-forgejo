# WEBHOOK-026 — récepteur et CLI de réconciliation en processus séparés

Date : 2026-10-22. Worktree : `local/WEBHOOK-005-canonical-links`. HEAD examiné : `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`.

## Parcours observé

Le test `actual_reconcile_cli_process_reads_receiver_journal_against_private_http_fixture` démarre le vrai binaire `forgejo-webhook-local`, accepte deux webhooks HMAC synthétiques pour une issue, arrête le processus, reprend le même journal sous un nouveau port, puis reconnaît une redelivery comme doublon sans ajouter d’événement. Il lance ensuite le vrai binaire `forgejo-reconcile-local` comme processus distinct. Le CLI lit les deux livraisons depuis le journal et fait un seul GET vers une fixture HTTP privée sur `127.0.0.1:3130`; le payload webhook indique `closed`, tandis que le snapshot courant fixture indique `open`. Les deux résultats rendent `current_state_read` avec l’état courant `open`; aucun en-tête Authorization n’est envoyé.

Le second test `actual_reconcile_cli_process_keeps_forbidden_upstream_inconclusive` suit la même réception de deux événements puis répond `403` au GET du vrai CLI. Le processus sort avec un statut d’échec et un JSON batch `inconclusive`, `upstream_forbidden`, non réessayable, sans `canonical_state`. Le corps d’erreur HTTP contient un marqueur synthétique et ce marqueur n’apparaît ni dans stdout ni stderr du CLI.

Le troisième test `actual_reconcile_cli_process_bounds_transient_upstream_retries` répond `503` trois fois au vrai CLI. Le journal de fixture observe exactement trois GET; la sortie reste `inconclusive`, porte `upstream_http_status`, `retryable=true` et `http_status=503`, sans état canonique. Le corps 503 synthétique n’est pas recopié dans la sortie.

Le quatrième test `interrupted_reconcile_cli_can_resume_from_the_unchanged_journal` intercepte le premier GET, puis termine par SIGTERM le groupe contenant le vrai CLI et son enfant curl. Les octets du journal restent identiques après l’interruption et après la reprise. Une nouvelle invocation du CLI reprend ce même journal, lit l’état courant `open` en un GET et rend les deux résultats canoniques. La fixture attend le GET avant l’interruption; sa barrière est libérée par le test après SIGTERM ou par la fermeture du canal si le test échoue, sans course contre le timeout HTTP de cinq secondes.

Le test attend un `405 method_not_allowed` propre au récepteur avant tout envoi HMAC. Si le processus sort, ne répond pas dans les trois secondes ou ne renvoie pas cette réponse JSON, le port est abandonné et l’allocation recommence, au plus cinq fois. La même sonde est utilisée après reprise du journal. Secret synthétique : mode `0600`; répertoire temporaire : `0700`; les enfants effacent leur environnement.

Le lanceur `scripts/run_webhook_reconcile_cli_e2e.py` compile le test en mode hors ligne et verrouillé dans un `CARGO_TARGET_DIR` temporaire neuf. Il exécute chacun des quatre tests ignorés dans son propre `/usr/bin/bwrap --unshare-net`, avec racine montée en lecture seule, `/tmp` en tmpfs, cible Cargo montée en lecture seule et environnement nettoyé. Il échoue si `bwrap` manque et journalise les commandes exactes. La phase de compilation `cargo test --no-run` se déroule hors de `bwrap`; l’exécution des tests et de leurs processus enfants est celle isolée du réseau.

La revue indépendante initiale avait relevé l’absence d’isolation réseau reproductible et une sonde readiness trop faible; le lanceur bwrap dédié et la réponse HTTP `405 method_not_allowed` les corrigent. Une revue du cas d’interruption a ensuite relevé que la comparaison de longueur du journal ne prouvait pas son immutabilité et que la barrière expirait avec le timeout HTTP. Le test compare maintenant les octets complets et libère la barrière sur signal ou fermeture du canal. La revue de suivi confirme que ces deux points sont corrigés et ne relève pas de défaut restant.

## Vérifications reproductibles

- `python3 scripts/run_webhook_reconcile_cli_e2e.py > evidence/webhook-receiver-cli-e2e-026-isolated.log 2>&1` : compilation neuve puis exécution des quatre tests sous `bwrap --unshare-net`; 4 réussis, 0 échec. Le journal contient les quatre lignes complètes de `bwrap`.
- `CARGO_TARGET_DIR=/tmp/webhook-receiver-adapter-025-clean-target cargo test --offline --locked --test webhook_receiver_process_e2e -- --nocapture` : 1 réussi, 0 échec, 4 tests ignorés (exécutés séparément sous `bwrap`).
- `CARGO_TARGET_DIR=/tmp/webhook-receiver-adapter-025-clean-target cargo test --offline --locked --all-targets` : 23 suites, 210 réussis, 0 échec, 6 ignorés. Quatre ignorés sont les parcours CLI à port fixe; deux autres sont des helpers de processus.
- `CARGO_TARGET_DIR=/tmp/webhook-receiver-adapter-025-clean-target cargo clippy --offline --locked --all-targets -- -D warnings -A clippy::too_many_arguments -A clippy::bool_assert_comparison -A clippy::manual_div_ceil` : réussi avec ces trois catégories de lints préexistantes neutralisées.
- `cargo fmt --all -- --check` et `git diff --check` : réussis. Un contrôle whitespace séparé couvre aussi les deux fichiers nouveaux ou non suivis par Git (`tests/webhook_receiver_process_e2e.rs` et `scripts/run_webhook_reconcile_cli_e2e.py`).

Journaux : `evidence/webhook-receiver-cli-e2e-026-{isolated,focused,all-targets,clippy,rustfmt,diff-check}.log`.

Empreintes SHA-256 des sources, du manifeste Cargo et des journaux : `evidence/webhook-receiver-cli-e2e-026-sha256.txt`.

Outils observés : Cargo 1.95.0, rustc 1.95.0, Python 3.12.3 et bubblewrap 0.9.0. Dépendances prises du `Cargo.lock` existant avec `--offline --locked`; aucun accès réseau de Cargo n’est demandé.

Surveillance indépendante au 2026-10-22 22:20:09 UTC : PID 70583 vivant et endormi; marqueur Ralph `active=true`, itération 5/8, session `c6bf73b1-7a36-49d0-bd29-88c564e8456c`. SHA-256 du module cible toujours `bf4187ad7ba6f797814940c1c6c524fe9335ba89f246c68a7e3b5644bd5fe1fc`. Le moniteur n’a pas consulté le terminal; un rapport de progression précédent indiquait une invite d’autorisation en attente, dont le contenu reste inconnu. Aucune réponse ni permission n’a été donnée.

## Portée et limites

Ce jalon ajoute un lanceur et des assertions de test seulement; aucun code de production n’a changé. Le récepteur et le CLI sont des processus réels du dépôt, mais l’API est une fixture locale synthétique, sans Forgejo live ni credential réel. Aucune PR, exécution CI Forgejo, vérification liée à une PR, décision de fusion ou LLM n’est exercé ici. Le test ne mesure aucun gain de qualité, coût ou délai et ne démontre pas la sécurité d’un déploiement.

Suite utile : examiner une vérification de PR dont les preuves lient exactement base et head, et le point d’intégration local qui relit ces versions au moment de la décision; garder les fixtures pour ce qui n’est pas autorisé ou accessible par l’API locale. Aucun credential provider hôte autorisé n’a été démontré dans ce lot.
