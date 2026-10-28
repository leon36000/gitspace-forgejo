# WEBHOOK-RETRY-SCHEDULE-001 — exposer le délai minimal de reprise

Date : 24 octobre 2026. Dépôt local `local/WEBHOOK-005-canonical-links`, HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`, workspace déjà fortement modifié. Aucun commit, push, publication, changement de production ou appel modèle.

## Lacune reproduite

Le client réconciliateur respectait un `Retry-After` valide tant que l’attente tenait dans son échéance. Si le serveur demandait 3600 secondes et le budget de lecture du CLI était de 30 secondes, il émettait une seule requête puis retournait un verdict inconclusif `upstream_http_status`, `retryable=true`, `http_status=429`, sans dire au processus appelant quand retenter. Un orchestrateur pouvait donc retenter immédiatement et ignorer la consigne du serveur.

Le test processus existant a d’abord été renforcé pour attendre `details.retry_after_ms=3_600_000`. Le runner isolé a échoué comme prévu : JSON `null` au lieu de la valeur attendue; les huit scénarios précédents dans ce runner avaient réussi. Détail : [red.log](red.log).

## Changement

`ApiReadError::HttpStatusWithRetryAfter` transporte un statut retryable et une valeur entière bornée. Le client n’en produit qu’après avoir reçu un `Retry-After` valide et avoir décidé de rendre l’erreur plutôt que de poursuivre les tentatives dans son échéance. Le sérialiseur CLI ajoute alors `error.details.retry_after_ms`; l’action conseille d’attendre au moins ce délai. Le calcul sature à `u64::MAX`, sans conversion susceptible de déborder. Les valeurs d’en-tête invalides ne donnent pas de délai et restent non reflétées; le corps HTTP n’est pas rendu. Les réponses retryables sans en-tête gardent le comportement de backoff local antérieur.

Les versions JSON annoncées restent single `2` / batch `3`; le champ est optionnel dans `details`, dont les clés sont spécifiques au code d’erreur. Ce choix vise une extension additive. En Rust, `ApiReadError` est un enum public : un consommateur externe qui fait un `match` exhaustif pourrait devoir ajouter la nouvelle variante. Aucun consommateur hors workspace n’a été vérifié.

## Vérifications

Versions : [tool-versions.txt](tool-versions.txt) — Rust/Cargo 1.95.0, Python 3.12.3 et Bubblewrap 0.9.0.

- Test rouge et correction reproduits par `python3 scripts/run_webhook_reconcile_cli_e2e.py`; le runner passe ensuite **12/12** tests contre des fixtures HTTP loopback sous `bwrap --unshare-net`, et construit les artefacts dans un répertoire `/tmp` isolé. Journal complet : [process-e2e.log](process-e2e.log).
- Le cas `Retry-After: 3600` avec budget de 30 s renvoie `retry_after_ms=3600000` en un GET. Une date HTTP valide en 2099, hors budget, expose un délai numérique supérieur à 30 s; une date invalide n’en expose pas; une date future dans le budget reste respectée avant retry. Les corps 429 synthétiques ne sont pas reproduits dans la sortie.
- `cargo test --offline --locked --bin forgejo-reconcile-local` : **100 réussis**, 0 échec. [Journal](reconcile-cli-tests.log).
- `cargo test --offline --locked --test webhook_reconcile_contract` : **21 réussis**, 0 échec. [Journal](reconcile-contract-tests.log).
- `cargo test --offline --locked --all-targets --no-run` : code 0; tous les bins et tests du workspace compilent avec la nouvelle variante de l’enum. Cette commande ne les exécute pas. [Journal](all-targets-no-run.log).
- `python3 scripts/test_cargo_license_state.py` : 5 réussis. `verify_cargo_license_state.py` relit `license=null`, `license_file=null`, `publish=[]`, décision `unselected`, droits `not_assessed`. Journaux : [tests](cargo-license-tests.log), [readback JSON](cargo-license-verify.json).
- `cargo fmt --all -- --check`, Clippy ciblé et `git diff --check` passent. [Rustfmt](rustfmt.log), [Clippy avec allowances](clippy-allowlist.log), [diff-check](diff-check.log).

Le Clippy strict sans allowances a échoué sur les lints préexistants `too_many_arguments` et `collapsible_if` dans des fichiers voisins (`agent_runtime.rs`, `forgejo.rs`, `mission_admission.rs`, `lib.rs`). Quand ces deux catégories ont été neutralisées, Clippy a signalé le nom `into_*` de la méthode de conversion ajoutée; celle-ci a été renommée `as_api_read_error`. La commande finale repasse avec seulement les deux allowances de base documentées ci-dessus. Les diagnostics sont dans [clippy.log](clippy.log).

## Portée et limites

**Testé :** le receiver et le CLI Rust réels avec le mode single, un serveur HTTP synthétique dans un espace réseau sans accès extérieur, ainsi que le contrat Rust du réconciliateur. **Non testé :** un serveur Forgejo réel, les réponses 429 d’une instance Forgejo, la présence de la nouvelle valeur dans chaque item du mode batch, le merge, un agent LLM, ou l’utilisation par un ordonnanceur tiers. Le délai d’une date HTTP dépend de l’horloge locale au moment du parse; le consommateur doit respecter au minimum le champ reçu et conserver son propre budget. Aucun gain de productivité ou réduction de coût n’est mesuré.
