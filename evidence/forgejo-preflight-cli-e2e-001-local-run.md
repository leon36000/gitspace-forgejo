# MERGE-003 — préflight CLI réel avec head/base/critères périmés

Date : 2026-10-22. Worktree : `local/WEBHOOK-005-canonical-links`. HEAD examiné : `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`.

## Parcours

`tests/forgejo_preflight_process_e2e.rs` signe une preuve de succès synthétique liée aux hashes d’exigence/politique, à un head et à une base connus. Il écrit la preuve, le binding attendu et la clé de vérification dans des fichiers privés temporaires; le processus CLI reçoit un environnement vidé.

Le test lance le vrai processus `forgejo-reconcile-local --preflight` cinq fois, contre une fixture HTTP loopback synthétique :

1. Base et head courants égaux au reçu : le CLI renvoie `proof_current`, avec `merge_authorized=false` et `merge_attempted=false`.
2. Base inchangée, head courant différent : code de sortie en échec et résultat `inconclusive/stale_candidate`.
3. Head inchangé, base courante différente : code de sortie en échec et résultat `inconclusive/stale_base`.
4. PR courante identique, mais hash d’exigences attendu modifié après signature : code de sortie en échec et résultat `inconclusive/stale_requirements`.
5. PR courante identique, mais hash de politique attendu modifié après signature : code de sortie en échec et résultat `inconclusive/stale_policy`.

La preuve reste signée sur les hashes initiaux `a…` et `b…`; les deux scénarios de critères modifient seulement le binding attendu lu par le CLI. Cela vérifie l’invalidation locale du reçu si les attentes changent, mais ne prouve pas que la source de ces attentes soit protégée contre leur auteur. Chaque invocation lit le chemin API GET attendu sans Authorization. Après la réponse, la fixture reste en écoute jusqu’au signal envoyé après la terminaison du CLI; toute requête supplémentaire est enregistrée et répond 405. Elle débloque aussi à la fermeture du canal si le test échoue et draine les connexions déjà en file au signal d’arrêt. Les cinq scénarios n’observent qu’un GET chacun, sans POST de fusion; même le cas courant répond `merge_authorized=false` et `merge_attempted=false`.

Le scénario de processus est ignoré dans le lancement Cargo ordinaire afin d’éviter le conflit du port 3130. Le test non ignoré `request_header_reader_preserves_a_pipelined_follow_up_request` vérifie que le parseur ne perd pas une seconde requête HTTP sur le même socket. `scripts/run_forgejo_preflight_cli_e2e.py` compile hors ligne/verrouillé dans une cible temporaire neuve puis lance ces deux tests dans des `/usr/bin/bwrap --unshare-net` séparés, avec `/tmp` tmpfs, cible montée en lecture seule et environnement nettoyé. Le fixture reste ouvert jusqu’à ce que le parent confirme la fin du CLI, examine les connexions sur la socket initiale et les nouvelles connexions, puis draine la file avant de s’arrêter.

## Vérifications reproductibles

- `python3 scripts/run_forgejo_preflight_cli_e2e.py > evidence/forgejo-preflight-cli-e2e-001-isolated.log 2>&1` : compilation temporaire et parcours isolés réussis 2/2 (scénario de cinq préflights et parseur HTTP pipeliné).
- `CARGO_TARGET_DIR=/tmp/webhook-receiver-adapter-025-clean-target cargo test --offline --locked --all-targets` : 24 suites, 211 réussis, 0 échec, 7 ignorés. Le scénario de processus fait partie des ignorés et est lancé séparément par le runner; le parseur est testé normalement.
- `CARGO_TARGET_DIR=/tmp/webhook-receiver-adapter-025-clean-target cargo clippy --offline --locked --all-targets -- -D warnings -A clippy::too_many_arguments -A clippy::bool_assert_comparison -A clippy::manual_div_ceil` : réussi avec trois catégories de lints préexistantes neutralisées.
- `cargo fmt --all -- --check`, `git diff --check` et le contrôle whitespace des deux nouveaux fichiers : réussite.

Vérification finale après consignation du checkpoint : rustfmt, diff-check et contrôle LF/fin de ligne ont été relancés avec succès. `sha256sum -c evidence/forgejo-preflight-cli-e2e-001-sha256.txt` valide chaque entrée du manifeste.

Un premier lancement all-targets a échoué dans le test préexistant `candidate_cannot_forge_libtest_success_with_print_and_process_exit` de `mission-012-verify-local`; le recheck après l’extension a reproduit le même problème. L’investigation a établi la cause : le test externe exigeait un texte de sortie que libtest peut ne pas vider après le `process::exit(0)` adversarial. Seules les assertions de ce test ont été corrigées; aucun code de production n’a changé. Les deux échecs restent conservés dans `forgejo-preflight-cli-e2e-001-all-targets-initial-failure.log` et `forgejo-preflight-cli-e2e-001-all-targets-stdout-capture-failure.log`. Les deux binaires ciblés réussissent ensuite, puis l’all-targets final passe 24/24 suites, 211/0/7. Détails : [rapport MISSION-012 stdout-capture](mission-012-stdout-capture-regression-local-run.md) et [revue indépendante](mission-012-stdout-capture-regression-review.md).

Les revues indépendantes ont signalé la fermeture trop précoce de la fixture, puis le risque de masquer une requête pipelinée sur le socket initial. Le fixture attend le signal de fin du processus, inspecte la socket initiale octet par octet et écoute les connexions nouvelles; un test non ignoré vérifie le découpage de deux en-têtes pipelinés et le runner l’exécute sous bubblewrap. La dernière revue confirme que le reçu conserve les anciens hashes, que le binding attendu change dans les deux scénarios, que les codes de refus correspondent et que le runner isole chacun des tests; aucun défaut actionnable restant n’est rapporté ([note de revue](forgejo-preflight-cli-e2e-001-review.md)). Les commandes complètes et sorties sont dans `evidence/forgejo-preflight-cli-e2e-001-*`; sources, rapport, revue, checkpoint et journaux sont couverts par `evidence/forgejo-preflight-cli-e2e-001-sha256.txt`.

## Limites

Ce lot n’ajoute que des tests et leur runner; aucun code de production ne change. Le binaire est réel, mais l’API, les hashes et les attentes modifiées sont synthétiques. Aucun Forgejo live/authentifié, changement réel de PR, vérificateur indépendant, revue humaine ou merge n’a été exercé. Le `proof_current` indique seulement que le préflight local accepte le binding fourni; il n’accorde pas d’autorité de fusion et ne ferme pas le TOCTOU d’un merge.

Suite utile : conserver le blocage du POST de fusion tant que l’autorité hôte pour les exigences/politiques et l’approbation humaine, ainsi que le modèle de sérialisation des mises à jour de base, ne sont pas résolus. Un test fixture n’établit pas l’identité d’un décideur réel.

Surveillance ponctuelle du 2026-10-22 22:28:41 UTC : le PID Claude Code 70583 reste vivant et endormi; le marqueur Ralph reste à 5/8, sans progrès observable, et le hash du module cible est inchangé (`bf4187ad7ba6f797814940c1c6c524fe9335ba89f246c68a7e3b5644bd5fe1fc`). Le terminal n’a pas été consulté; une invite éventuelle reste inconnue. Aucune permission n’a été acceptée.
