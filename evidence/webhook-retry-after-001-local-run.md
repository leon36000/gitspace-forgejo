# WEBHOOK-RETRY-AFTER-001 — respect de `Retry-After`

Date : 24 octobre 2026. Worktree local `local/WEBHOOK-005-canonical-links`, HEAD examiné `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`; modifications non commitées préservées. Rust/Cargo 1.95.0.

## Changement

L’adaptateur HTTP loopback de `src/bin/forgejo-reconcile-local.rs` récupère maintenant `Retry-After` depuis la réponse curl et traite les formes délai en secondes et date HTTP. Le parseur accepte les trois syntaxes de date HTTP de RFC 9110. Un délai valide est respecté seulement s’il tient dans l’échéance monotone restante; sinon l’adaptateur retourne le statut transitoire reçu sans émettre une autre requête. Un champ présent mais invalide ne déclenche pas le backoff local; un champ absent garde le backoff borné existant. La règle de protocole est décrite à [RFC 9110 §10.2.3](https://www.rfc-editor.org/rfc/rfc9110.html#section-10.2.3) et les formes de date à [§5.6.7](https://www.rfc-editor.org/rfc/rfc9110.html#section-5.6.7).

Le test loopback couvre un 429 avec `Retry-After: 30` et seulement 500 ms de budget : une requête est envoyée, aucune seconde lecture n’est faite, et le résultat reste `HttpStatus(429)`. Le test du parseur couvre les secondes, les trois formes de date, une date passée, un champ absent et une valeur invalide. Le test rouge initial avait reproduit la seconde requête indue avant le changement; son affichage n’a pas été conservé dans un journal séparé.

## Vérifications

| Commande | Résultat |
| --- | --- |
| `cargo test --offline --locked --bin forgejo-reconcile-local` | 98 réussis, 0 échec, 0 ignoré. [Journal](webhook-retry-after-001-focused.log) |
| `cargo test --offline --locked --all-targets -- --test-threads=4` | 34 groupes, 424 réussis, 0 échec, 20 ignorés. [Journal](webhook-retry-after-001-all-targets.log) |
| `cargo fmt --all -- --check` | Passe. |
| `git diff --check` | Passe. |
| `cargo clippy --offline --locked --all-targets -- -D warnings -A clippy::too_many_arguments -A clippy::bool_assert_comparison -A clippy::manual_div_ceil -A clippy::collapsible_if -A clippy::enum_variant_names` | Passe avec cinq catégories de lint neutralisées. [Journal](webhook-retry-after-001-clippy-five-baselines.log) |

Le Clippy avec les trois allowances antérieures a échoué sur deux `collapsible_if` dans le fichier local non suivi `src/agent_runtime.rs`; avec ces deux warnings neutralisés, il a relevé `enum_variant_names` dans `tests/forgejo_merge_post_crash_recovery_process_e2e.rs:300`. Ces lints n’ont pas été modifiés dans ce lot. Les diagnostics sont conservés dans [le premier journal](webhook-retry-after-001-clippy.log) et [le second](webhook-retry-after-001-clippy-baseline-allowances.log).

## Portée et limites

Cette vérification exerce le client curl réel contre des fixtures HTTP locales dans les tests Rust; elle ne démontre pas qu’une instance Forgejo v17 émet effectivement cet en-tête, et aucun nouveau parcours Forgejo de bout en bout n’a été relancé pour ce changement. Aucun agent LLM n’a été invoqué et aucun coût modèle nouveau n’a été engagé. Les modifications restent locales et non commitées.

## Vérification du processus receiver → CLI

Une tranche d’intégration supplémentaire relie le receiver réel à son journal puis au vrai `forgejo-reconcile-local`. Le scénario `actual_reconcile_cli_process_respects_retry_after_beyond_read_budget` est exécuté dans le runner `scripts/run_webhook_reconcile_cli_e2e.py`, sous `bwrap --unshare-net` (Bubblewrap 0.9.0). La fixture HTTP sur le loopback isolé renvoie `429` avec `Retry-After: 3600`, alors que la politique CLI annonce 30 secondes de budget de lecture. Le processus termine avec une seule lecture, un résultat individuel `inconclusive/upstream_http_status`, `retryable=true`, aucun état canonique et sans recopier le corps HTTP. La fixture reste ouverte une seconde après la première réponse pour capturer un retry par le backoff antérieur. Les neuf scénarios du runner passent : [journal](webhook-retry-after-001-process-e2e.log).

Vérification fraîche après cet ajout : `cargo test --offline --locked --all-targets -- --test-threads=4` — 34 groupes, 424 réussis, 0 échec, 21 ignorés; le nouveau processus E2E reste ignoré en appel Cargo ordinaire mais est explicitement lancé par le runner. Clippy passe avec les cinq allowances précisées ci-dessus, rustfmt, diff-check et `py_compile` passent. Les cinq tests de garde Cargo passent et le vérificateur readback confirme `license=null`, `license_file=null`, `publish=[]`, décision `unselected`, droits de redistribution `not_assessed`; aucun fichier ou identifiant de licence n’est choisi. Journaux : `webhook-retry-after-001-process-{all-targets,clippy,rustfmt,diff-check,pycompile}.log` et `webhook-retry-after-001-cargo-license-{tests,verify}.log`; état Cargo : `webhook-retry-after-001-cargo-metadata-state.log`.

Deux premiers essais du nouvel E2E ont détecté des attentes incorrectes dans le test : le résultat d’une livraison est le schéma v2 au niveau supérieur, tandis que le schéma v3 `results[]` sert aux lots. Ces journaux sont conservés dans `webhook-retry-after-001-process-e2e-{initial-assertion-failure,results-shape-failure}.log`; le code de production n’a pas changé dans cette tranche.

Limite : ce parcours traverse les processus et `curl` contre une fixture synthétique, pas une instance Forgejo réelle; la preuve de livraison n’est pas une validation en usage réel. Aucun LLM ni dépense nouvelle.


## Addendum WEBHOOK-RETRY-AFTER-003 — dates HTTP et en-têtes invalides au niveau processus

Date : 24 octobre 2026. Sources de test non suivies Git dans le worktree, basé sur HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0` : `tests/webhook_receiver_process_e2e.rs` SHA-256 `c4ab0202701b2276fb7e3fbfec853bd56f09bf071b200f772ed742339c6bd2c8`; `scripts/run_webhook_reconcile_cli_e2e.py` SHA-256 `d95c6ca2a4fc7aff323db41b75bb67c885f0f984601c2dda33f4474da42f1481`.

Le test processus démarre le receiver réel, signe et journalise un événement, puis appelle le CLI de réconciliation réel. Une fixture HTTP loopback émet : (1) `Retry-After: tomorrow`, attendu comme invalide, une lecture et aucun retry de backoff pendant l’observation; (2) une date valide de 2099 au-delà du budget de 30 s, une lecture et aucune attente impossible; (3) une date HTTP générée au moment de la réponse 429, à 3 s dans le futur, trois tentatives bornées et un délai mesuré réponse→connexion suivante d’au moins 2 s. Dans chaque cas d’échec amont, le résultat reste inconclusif (`upstream_http_status`), sans état canonique ni fuite du corps HTTP. Le dernier cas distingue le parse de date valide d’un fallback court.

La revue statique indépendante a d’abord relevé qu’une date future très lointaine pouvait être rejetée comme invalide tout en satisfaisant l’assertion « une requête ». Le test a été renforcé : date générée à la première réponse et mesure directe jusqu’à la connexion suivante. La revue finale ne trouve aucun défaut matériel actionnable; elle n’a pas exécuté les tests.

Vérifications fraîches : le cas date HTTP ciblé passe en 5,31 s; le runner complet passe **10/10** scénarios sous `bwrap --unshare-net`; `cargo test --offline --locked --all-targets -- --test-threads=4` passe avec 34 groupes, **424 réussis, 0 échec, 22 ignorés**; Clippy passe avec les cinq allowances documentées plus haut; `cargo fmt --all -- --check`, `git diff --check` et `py_compile` passent. Voir les journaux `webhook-retry-after-003-{http-date-focused-final,process-e2e-final2,all-targets-final2,clippy-final2,rustfmt-final2,diff-check-final2,pycompile-final2}.log` et les empreintes de `evidence/SHA256SUMS`.

Portée : cette tranche démontre le comportement processus contre un serveur HTTP synthétique isolé, pas contre Forgejo v17 en direct. Elle n’emploie pas de LLM et ne mesure aucun gain général. Les premiers journaux en échec restent conservés : ils documentent les assertions de fixture trop optimistes et leurs corrections, pas un défaut établi du service. Aucun changement n’est commité ou publié.


## Addendum WEBHOOK-OUT-OF-ORDER-001 — état canonique hors ordre et échéance HTTP partagée

Date : 24 octobre 2026. Worktree `/home/pc6/Documents/Codex/2026-10-21-mission-construire-une-forge-con-ue/.runtime/webhook-005-workspace`, HEAD examiné `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`; fichiers de test locaux non suivis par Git, aucun commit créé. SHA-256 : `tests/webhook_receiver_process_e2e.rs` `0b232a5c494db06d5952921816c6d45a08f14d8e6b3ff8d5d85c33c356060987`; `scripts/run_webhook_reconcile_cli_e2e.py` `9498d6b380c9ba8d1e44b0db6a6449bb92811e981a4bbbf53eadfd74ecb297e9`.

Le test processus d’ordre inverse livre au receiver réel `open@17:00` puis `closed@16:00`, demande les deux événements au CLI en ordre inverse et reçoit l’état canonique HTTP `closed@18:00`. Il vérifie la correspondance de chaque résultat à son `delivery_id`, une lecture de ressource, et un journal inchangé.

Le test de budget soumet neuf événements signés, laisse répondre les sept premières ressources, puis suspend la huitième ressource jusqu’à la limite de lecture CLI de 30 s. La fixture accepte les connexions en parallèle, horodate les en-têtes HTTP complets et reste active jusqu’à la sortie ou l’arrêt borné du groupe CLI; elle draine ensuite les connexions déjà en file. Le processus reste inconclusif, conserve le journal, borne le nombre de retries, ne lit aucune neuvième ressource et n’émet aucun GET complet après `started + 30 s`. Les premières tentatives du test ont révélé que `upstream_read_attempts` compte les retries HTTP et non les seules ressources; l’assertion a été alignée sur cette distinction, sans changement du code de production.

Vérifications fraîches sur cet état : runner sous `bwrap --unshare-net` **12/12** scénarios; `cargo test --offline --locked --all-targets -- --test-threads=4` : 34 groupes, **424 réussis, 0 échec, 24 ignorés**; Clippy passe avec les cinq allowances existantes documentées dans les sections précédentes; `cargo fmt --all -- --check`, `git diff --check` et `python3 -m py_compile scripts/run_webhook_reconcile_cli_e2e.py` passent. Journaux : `webhook-out-of-order-001-process-e2e-final5.log`, `webhook-out-of-order-001-shared-budget-focused-final5.log`, `webhook-out-of-order-001-all-targets-final5.log`, `webhook-out-of-order-001-clippy-final5.log`, `webhook-out-of-order-001-rustfmt-final5.log`, `webhook-out-of-order-001-diff-check-final5.log` et `webhook-out-of-order-001-pycompile-final5.log`; les essais rouges précédents restent conservés et hashés.

La revue statique indépendante n’a trouvé aucun défaut matériel résiduel. Elle note que l’horloge de test commence juste avant le spawn CLI, donc son seuil est légèrement conservateur par rapport au démarrage interne du budget; les requêtes lentes de cette fixture restent éloignées de cette frontière. Portée : fixture HTTP synthétique dans un namespace réseau privé, pas d’instance Forgejo réelle ni de LLM; aucun gain de productivité mesuré. Aucun changement de production, commit ou publication dans ce lot.
