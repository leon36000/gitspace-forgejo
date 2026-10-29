# MISSION-REQUIREMENT-PR-HEAD-RACE-001 — dernière relecture de la PR source

Date : 28 octobre 2026. Worktree produit fondé sur `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`, fortement modifié. Aucun dépôt distant ni service externe n’a été contacté.

## Défaut reproduit et correction

Le chemin schema v3 de `mission_admission::evaluate_requirement_arbitration` lisait la PR, lisait un snapshot complet des revues, relisait la PR, puis lisait un second snapshot complet des revues. Il retournait la décision sans relecture PR finale. Le test processus place le changement de `head.sha` exactement après le second snapshot. Avant le correctif, le vrai binaire local acceptait la décision, revendiquait la génération 2 et modifiait le journal alors que le head avait changé; le journal rouge détaillé est `red.log`.

Le code relit maintenant la PR après le second snapshot et compare le DTO complet (`scope`, `merged`, branche de base, head et commit de fusion) à la première lecture. Une dérive donne `mission_requirement_review_snapshot_changed` avant l’allocation durable. Le test vert observe les têtes `2222…` puis `4444…`, deux listes de revues, une troisième lecture PR finale, et un journal d’exécution identique octet pour octet. Résultat processus 1/1 : `green-final2.log`.

Le banc existant était obsolète par rapport au code source actuel : son origine fixe avait déménagé du binaire au catalogue `mission_profile.rs`, `include_bytes!` exige un fixture de requirements copié, et le parseur d’arbre exige son SHA racine. Le test copie désormais ce seul fixture et réécrit l’origine uniquement dans sa copie temporaire. Aucun de ces ajustements ne modifie le runtime produit.

## Vérifications

- Test processus Forgejo-adapter avec API HTTP loopback synthétique : 1 réussi; il couvre aussi policy absente et reviewer synthétique non autorisé. Le runtime agent n’est pas invoqué.
- `cargo test --locked --offline --test mission_requirement_arbitration_contract -- --test-threads=1` : 13/13.
- `cargo test --locked --offline --all-targets -- --test-threads=1` : 765 réussis, 0 échec, 200 ignorés, 38 groupes. Les 200 ignorés ne sont pas comptés comme exercés.
- `cargo fmt --all -- --check`, `git diff --check` et `scripts/verify_cargo_license_state.py` passent. Cargo conserve `license=null`, `license_file=null`, `publish=[]`, décision `no_license_selected`, droits `not_assessed`.
- `cargo clippy --locked --offline --all-targets -- -D warnings` échoue sur 24 diagnostics Clippy existants ailleurs dans l’arbre (notamment signatures longues et styles dans `agent_runtime.rs`, plusieurs fonctions déjà présentes de `mission_admission.rs` et `webhook/reconcile.rs`). Aucun diagnostic ne vise le nouveau bloc de relecture finale; le journal complet est `clippy.log`.
- Après l’ajout des liens de ce reçu au checkpoint et à la matrice, le vérificateur documentaire a relu 1 605 liens locaux dans les cinq documents canoniques et n’a trouvé aucune cible manquante (`documentation-links-after.json`).

## Portée et réserves

Le parcours est un vrai processus Rust compilé hors ligne depuis une copie temporaire du package, face à une fixture HTTP sur `127.0.0.1`; PR, avis, identifiants et commits sont synthétiques. Il ne démontre ni un événement webhook natif, ni une instance Forgejo, un agent LLM/AgentOS, une fusion, ou un gain comparatif. Les relectures sont non transactionnelles : le contrôle final réduit cette fenêtre de course, mais Forgejo ne fournit toujours pas de snapshot atomique, et les refs ou avis peuvent changer après la dernière lecture. Ce lot n’autorise donc pas une assertion de CAS serveur.

Versions, empreintes sources, commandes et journaux sont conservés dans ce dossier; son manifeste de fichiers est `SHA256SUMS`.
