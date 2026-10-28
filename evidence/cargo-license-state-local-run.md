# CARGO-LICENSE-STATE-001 et WEBHOOK-REDIRECT-GUARD-001

Date : 2026-10-22. Worktree local : `local/WEBHOOK-005-canonical-links`, HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`; `origin/main` local observé à sept commits d’avance. Le worktree était déjà fortement modifié; aucun reset, nettoyage, rebase, commit ou publication n’a été fait. `Cargo.toml` conserve `publish = false` et n’a ni `license` ni `license-file`.

## Décision Cargo : licence non choisie

La documentation Cargo décrit `license` comme la licence sous laquelle le logiciel est distribué, `license-file` comme le chemin du texte de licence et `publish=false` comme une façon explicite d’empêcher la publication. Le manifeste ne peut donc pas résoudre l’indécision en ajoutant un identifiant ou un fichier sans faire une déclaration que le détenteur des droits n’a pas choisie. La valeur omise se voit comme `null` dans le JSON Cargo; `publish=[]` reflète la publication désactivée. Sources primaires : [champs licence du manifeste Cargo](https://doc.rust-lang.org/cargo/reference/manifest.html#the-license-and-license-file-fields), [champ publish](https://doc.rust-lang.org/cargo/reference/manifest.html#the-publish-field) et [format `cargo metadata`](https://doc.rust-lang.org/cargo/commands/cargo-metadata.html).

`scripts/verify_cargo_license_state.py` appelle `cargo metadata --locked --offline --format-version 1 --no-deps`, vérifie qu’il existe exactement un package `agent-forge-core`, puis exige `license=null`, `license_file=null` et `publish=[]`. Il émet `license_decision=unselected` et `redistribution_rights=not_assessed`; ce rapport ne prétend ni choisir une licence ni autoriser la redistribution. Cinq tests Python standard refusent un identifiant de licence, un fichier, la réactivation de la publication et les métadonnées absentes/ambiguës. Résultats : [readback courant](cargo-license-state-check.json), [tests](cargo-license-state-tests.log).

Cela ne tranche pas la propriété du code, le droit de redistribution ni les licences des dépendances épinglées. La décision du détenteur des droits et l’audit juridique de l’assemblage restent nécessaires avant redistribution. Aucun fichier `LICENSE` ni champ de licence n’a été ajouté; aucune commande de publication n’a été lancée.

## Garde contre les redirections avec credential

Dans `src/bin/forgejo-reconcile-local.rs`, `start_redirect_http_fixture` répond 302 vers une seconde route du même listener loopback. `local_adapter_does_not_follow_redirects_with_host_credential` passe un jeton synthétique au fournisseur test-only et vérifie un résultat `HttpStatus(302)`, une seule requête acceptée et l’en-tête seulement sur la requête d’origine. La fixture sert du JSON valide sur une seconde requête, ce qui rend le suivi observable et empêche un faux positif dû à un échec de connexion.

Le test a été contrôlé par injection temporaire de `--location` dans la commande curl : il a échoué comme attendu en retournant le JSON `{"number":12}` de la seconde réponse au lieu de `HttpStatus(302)`. L’option a été retirée avant les vérifications finales; le constructeur utilise toujours `/usr/bin/curl` sans option de suivi. La revue indépendante a signalé que la fixture supposait à tort que les en-têtes arrivent dans un seul `read`; elle lit maintenant jusqu’au séparateur `\r\n\r\n`, avec une limite de 16 KiB et un timeout. Le test vert final est dans [webhook-redirect-guard-focused.log](webhook-redirect-guard-focused.log). La vérification d’origine et l’absence de lecture de credential pour une autre origine restent dans [le rapport du seam précédent](webhook-host-credential-001-local-run.md).

Ce test utilise un processus `/usr/bin/curl`, un secret synthétique et deux échanges HTTP loopback; il ne contacte pas Forgejo. La revue statique indépendante du delta n’a exécuté ni test ni API. Le chemin credential reste compilé sous `#[cfg(test)]`; le CLI de production reste anonyme. HTTP sur loopback n’est pas adéquat pour un secret réel, et aucun secret existant n’a été lu.

## Vérifications finales

- `python3 -m unittest -v scripts/test_cargo_license_state.py` : 5 réussis; journal ci-dessus.
- `python3 scripts/verify_cargo_license_state.py` : `status=verified`, `license_decision=unselected`, `license=null`, `license_file=null`, `publish=[]`, `redistribution_rights=not_assessed`.
- `cargo test --locked --offline --bin forgejo-reconcile-local local_adapter_does_not_follow_redirects_with_host_credential -- --exact tests::local_adapter_does_not_follow_redirects_with_host_credential --nocapture` : 1 réussi.
- `cargo test --locked --offline --all-targets` : 23 suites, 208 réussis, 0 échec, 2 helpers ignorés; [journal](webhook-redirect-guard-all-targets.log).
- `cargo build --locked --offline --release --all-targets` : code de sortie 0; [journal](webhook-redirect-guard-release-build.log).
- `cargo fmt --all -- --check` et `git diff --check` : code de sortie 0; [journaux format](webhook-redirect-guard-rustfmt.log) et [diff](webhook-redirect-guard-diff-check.log).
- `cargo clippy --locked --offline --all-targets -- -A clippy::suspicious_open_options -A clippy::bool_assert_comparison` : code de sortie 0; les avertissements historiques `too_many_arguments` et `manual_div_ceil` restent visibles dans [le journal](webhook-redirect-guard-clippy.log). Ce n’est pas un Clippy strict sans avertissement.

Le parcours MISSION-012 décrit ailleurs dans `evidence/mission-012-local-run.md` est un unique cas local Claude Code → PR → vérification indépendante → fusion contrôlée. Ces deux sous-lots ne réexécutent pas ce parcours et ne démontrent ni une amélioration comparative, ni une intégration AgentOS/Ollama en production. Ils ne modifient aucun état Forgejo.
