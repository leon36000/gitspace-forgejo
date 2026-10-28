# WEBHOOK-HOST-CREDENTIAL-001 — fournisseur de credential réservé aux tests

Date : 2026-10-22. Worktree : `local/WEBHOOK-005-canonical-links`; HEAD examiné `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`; `origin/main` local `6030e13e3887926f8629cbb22f785d5b4bab580a` (HEAD est à sept commits derrière). `rustc 1.95.0 (59807616e 2026-04-14)`, `cargo 1.95.0 (f2d3ce0bd 2026-03-21)`. Le worktree était déjà fortement modifié avant ce lot; les fichiers existants n’ont été ni nettoyés, ni rebasés, ni commités.

## Portée et décision

WEBHOOK-MISSION-ADMISSION-002 testait admission et replay contre des fixtures, sans lecture Forgejo authentifiée. Ce lot vérifie un seam minimal de fourniture hôte à l’aide d’un jeton synthétique, ainsi que le refus d’une destination étrangère avant même l’accès au credential. L’analyse ne démontre aucun fournisseur activé en production, aucune lecture Forgejo privée et aucun accès à AgentOS, Forge Code, RAG, mémoire ou à un modèle.

La revue statique indépendante a découvert que le fournisseur était d’abord consulté avant que le constructeur d’URL ne rejette une destination étrangère. Un test compteur a reproduit cette lecture (`calls=1` au lieu de `0`); `read_current_once` valide maintenant la destination avant le fournisseur, et `build_curl_request` répète la validation comme défense en profondeur. La première version du test d’origine avait aussi échoué avant le garde-fou en recevant l’état `200` du serveur étranger au lieu de `Forbidden`. Les sorties rouges ont été observées lors des commandes interactives, mais ne sont pas conservées dans un journal séparé.

## Changements et comportement observé

- `src/bin/forgejo-reconcile-local.rs:322` vérifie que l’URL conserve exactement l’origine attendue et qu’elle est suivie d’un seul `/`; `//`, antislash, CR, LF, NUL et fragment sont refusés. `:381` applique le contrôle avant de construire curl. `:473` le répète avant d’interroger le fournisseur de credential.
- `:1522` vérifie que le jeton synthétique n’apparaît pas dans les arguments ni les variables d’environnement de curl; sa configuration `Authorization` est transmise par stdin. `:1551` couvre jeton vide, surdimensionné, mal paddé et injection d’en-tête. `:1586` vérifie le refus d’un faux serveur loopback d’une autre origine, zéro lecture du fournisseur et zéro requête reçue. `:1810` vérifie que l’erreur de credential est actionnable, non réessayable et sans valeur secrète dans le message.
- La commande curl est construite avec environnement vidé, sans proxy, HTTP seulement, sans redirection et avec une réponse bornée. Le test positif utilise un vrai processus `/usr/bin/curl` et un faux serveur TCP loopback local; ce n’est pas une instance Forgejo.
- L’API Forgejo documente l’en-tête `Authorization: token …` et le schéma `Bearer` dans [API Usage](https://forgejo.org/docs/latest/user/api/usage/). La documentation de portée Forgejo v17 décrit des jetons de lecture (`read:issue`, `read:repository`) et des jetons limités à un dépôt dans [Access Token Scope](https://forgejo.org/docs/v17.0/user/authentication/token-scope/). Ces pages documentent le protocole et la politique de scope; ce lot n’a pas testé une combinaison de scopes contre une version Forgejo ni validé un jeton réel.

## Vérifications reproductibles

Toutes les commandes Cargo ci-dessous ont été exécutées avec `--locked --offline` dans le worktree local. Le test d’origine utilise exclusivement une fixture loopback. Aucun service Forgejo, API privée, credential existant, modèle ou service externe n’a été appelé.

- `cargo test --locked --offline --bin forgejo-reconcile-local local_adapter_rejects_foreign_request_url_before_sending_host_credential -- --nocapture` : 1 réussi; couvre rejet avant accès credential et réseau.
- `cargo test --locked --offline --bin forgejo-reconcile-local synthetic_credential_is_not_added_to_curl_arguments_or_environment -- --nocapture` : 1 réussi.
- `cargo test --locked --offline --all-targets` : 23 suites, 207 réussis, 0 échec, 2 helpers ignorés; [journal](webhook-host-credential-001-all-targets.log).
- `cargo build --locked --offline --release --all-targets` : code de sortie 0; [journal](webhook-host-credential-001-release-build.log).
- `cargo fmt --all -- --check` : code de sortie 0; [journal](webhook-host-credential-001-rustfmt.log).
- `git diff --check` : code de sortie 0; [journal](webhook-host-credential-001-diff-check.log).
- `cargo clippy --locked --offline --all-targets -- -A clippy::suspicious_open_options -A clippy::bool_assert_comparison` : code de sortie 0, mais avertissements `too_many_arguments` et `manual_div_ceil` subsistent dans le code existant; [journal](webhook-host-credential-001-clippy.log). Ce n’est pas un résultat sans avertissement.

La revue indépendante a inspecté les frontières d’origine, l’ordre de validation, argv/env/stdin et le statut test-only. Après correction de l’ordre, elle n’a pas relevé d’autre défaut concret dans cette portée. C’était une revue statique seulement, sans exécution de test, service ou API.

## Limites et suite

`ForgejoApiToken::parse`, son plafond et `with_host_credentials` sont réservés à `#[cfg(test)]`; le chemin CLI normal garde le fournisseur à `None`. La transmission testée demeure HTTP sur loopback et n’offre aucune isolation contre un observateur local ou les secrets en mémoire. Il serait inapproprié d’y brancher un vrai secret. Avant une lecture authentifiée, il faut établir une source de credential hôte autorisée, sa portée minimale, son cycle de révocation et un transport adapté; l’accès anonyme précédent au service privé local était refusé (403) et n’a pas été contourné. La prochaine action documentée reste l’inspection de cette source et d’un binding protégé issue→exigence, puis seulement un test local explicitement autorisé.

Ce lot n’est pas le parcours de bout en bout exigé par la mission : aucune demande n’a été traitée par un agent LLM, aucun changement de dépôt n’a traversé une PR ni une vérification indépendante, et aucune fusion n’a été tentée. Le test synthétique prouve le contrat du seam, pas un gain de qualité/coût/délai.

Empreintes SHA-256 finales :

- `src/bin/forgejo-reconcile-local.rs` : `f224caabfd8de9fba60b55e3b1ab4a18c18cfe278776a1f635e5a773e42ad11d`
- suite all-targets : `b677cef6604e232930cfb62f324fa6ac6cd0ffb4e5fe229ae0715b0ae70e7729`
- build release : `8885782a30612fd0f508cbc032d7ac385c56c2e3815382a22deb34cef5d7addb`
- Clippy : `652bc1c724d45ccad06b5a69fe30c48d0e4162fe2f363f2604a0527e1596719c`
- rustfmt et `git diff --check` : sortie vide, SHA-256 `e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855`
