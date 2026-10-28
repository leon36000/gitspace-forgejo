# WEBHOOK-ADAPTER-RECONCILE-E2E-001

Date : 2026-10-22. Worktree : `local/WEBHOOK-005-canonical-links`, HEAD observé `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`, `origin/main` local est 7 commits devant. Le worktree était déjà fortement modifié; aucun reset, nettoyage, rebase, commit, push ou merge n’a été fait.

## Parcours exercé

Le test `tests::signed_journal_deliveries_reconcile_through_the_local_http_adapter` dans `src/bin/forgejo-reconcile-local.rs` signe deux livraisons Forgejo synthétiques portant un état `closed`, les ajoute au vrai `EventJournal` via `ingest_forgejo_delivery`, puis relit le journal. Le vrai `LocalLoopbackForgejoApi` effectue un GET avec `/usr/bin/curl` vers une fixture HTTP loopback qui renvoie l’état courant `open`. Le jeton est synthétique et injecté seulement par le fournisseur réservé aux tests.

Les deux IDs de livraison distincts produisent chacun un résultat `open`, avec numéro, horodatage et URL canonique attendus. Le cache de réconciliation partage le GET : le compteur d’appels du client vaut 1 et le serveur fixture reçoit un seul GET. La fixture lit les en-têtes jusqu’au séparateur HTTP avec limite et délais bornés. Le test couvre ainsi, dans un même processus de test, la vérification HMAC, l’append et le replay du journal, le client HTTP réel et la réconciliation de l’état courant; il ne passe pas par le serveur HTTP webhook.

Contrôle de sensibilité : une mutation temporaire dans `src/webhook/reconcile.rs` a écrasé l’état courant lu par l’adaptateur avec `closed`. Le test a échoué sur `left: "closed"`, `right: "open"`, code Cargo 101. La source de production a été restaurée byte pour byte avant la vérification finale. La sortie du test mutant est dans [webhook-adapter-reconcile-e2e-mutation.log](webhook-adapter-reconcile-e2e-mutation.log); le test vert après restauration est dans [webhook-adapter-reconcile-e2e-focused.log](webhook-adapter-reconcile-e2e-focused.log).

Une revue indépendante en lecture seule n’a trouvé aucun défaut concret dans le cache, le cycle de vie ou les assertions. Elle confirme les limites suivantes : le test n’exerce pas l’extraction HTTP des en-têtes ni le processus `forgejo-webhook-local`; le jeton n’est pas validé par Forgejo; les deux IDs sont distincts, donc ce cas ne rejoue pas la même livraison ni un redémarrage de processus. Le test e2e distinct du récepteur couvre déjà le processus HTTP, mais l’ensemble ne constitue pas une seule exécution cross-processus.

## Vérifications

- Test focalisé : `cargo test --locked --offline --bin forgejo-reconcile-local signed_journal_deliveries_reconcile_through_the_local_http_adapter -- --exact tests::signed_journal_deliveries_reconcile_through_the_local_http_adapter --nocapture` — 1 réussi.
- Suite : `cargo test --locked --offline --all-targets` — 23 suites, 209 réussis, 0 échec, 2 helpers ignorés; [journal](webhook-adapter-reconcile-e2e-all-targets.log).
- `cargo build --locked --offline --release --all-targets` — code de sortie 0; [journal](webhook-adapter-reconcile-e2e-release-build.log).
- `cargo fmt --all -- --check` — code de sortie 0; [journal](webhook-adapter-reconcile-e2e-rustfmt.log).
- `cargo clippy --locked --offline --all-targets -- -A clippy::suspicious_open_options -A clippy::bool_assert_comparison` — code de sortie 0, avec les avertissements existants `too_many_arguments` et `manual_div_ceil`; ce n’est pas un Clippy sans avertissement. [Journal](webhook-adapter-reconcile-e2e-clippy.log).
- `git diff --check` — code de sortie 0 après édition du rapport et du checkpoint; [journal](webhook-adapter-reconcile-e2e-diff-check.log).

## Portée et suite

Ce résultat exerce le vrai binaire curl local mais seulement avec un faux serveur loopback, des livraisons signées de fixture et un credential synthétique. Il ne démontre ni une lecture API Forgejo authentifiée, ni un déclenchement webhook cross-processus jusqu’à la réconciliation, ni une admission live, AgentOS ou un nouvel appel de modèle. Le secret de test n’est pas une autorité d’accès; aucun secret réel, réseau externe ou instance Forgejo n’a été utilisé.

La prochaine étape utile est un test cross-processus de bout en bout reliant le processus récepteur au client d’adaptation sans desserrer son allowlist d’origine et sans entrer en collision avec Forgejo. Si cette frontière exige d’extraire le client du binaire vers une interface réutilisable, le faire comme sous-lot distinct, avec une fixture limitée aux tests et sans fournisseur de credential de production tant que sa provenance et son transport ne sont pas autorisés. Le parcours comparatif sur plusieurs tâches reste à faire; MISSION-012 n’est qu’un cas local, `n=1`.
