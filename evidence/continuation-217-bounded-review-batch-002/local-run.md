# CONTINUATION-217 — lot maximal de notifications de revue

Date : 27 octobre 2026. Complément contractuel au [lot de troncature et versionnement](../continuation-217-bounded-review-notification-001/local-run.md). Le worktree est `.runtime/webhook-005-workspace`, HEAD de base examiné `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`, et ses changements préexistants sont conservés.

## Scénario

Le test `maximal_batch_of_large_review_notifications_stays_within_projected_output_budget` construit le maximum courant de 100 deliveries distinctes de commentaire de revue, chaque corps source ayant 128 KiB (13 107 200 octets au total). Le réconciliateur utilise un readback PR mocké; chaque résultat expose le préfixe 8 192 octets, la troncature, la longueur 131 072 octets et le digest SHA-256 de tout le texte source. Un seul GET PR est partagé entre les 100 notifications.

`reconcile_inbox_events` retourne les 100 résultats au lieu de refuser le budget de sortie de 8 MiB. Ce test confirme le chemin de projection et son budget dans une fixture de contrat; il ne démarre pas de serveur Forgejo et ne teste pas l’admission HTTP/HMAC des 100 corps.

## Vérifications

- Test ciblé : `cargo test --locked --offline --test webhook_reconcile_contract maximal_batch_of_large_review_notifications_stays_within_projected_output_budget -- --exact --nocapture` — 1 réussi.
- Suite : `cargo test --locked --offline --all-targets -- --test-threads=1` — code 0, 670 réussis, 0 échec, 124 ignorés, 36 résumés de cibles.
- `cargo fmt --all -- --check` et `git diff --check` passent.

Journal complet et compte exact : `rust-all-targets.log`, `rust-all-targets-summary.json`; commande et empreinte du test : `commands.txt`, `source-sha256.txt`. L’environnement Rust/Cargo 1.95.0 est consigné dans le reçu parent CONTINUATION-217.

## Réserves

La fixture utilise le même hôte et un `FixtureApi` déterministe. La note reste non fiable; le hash est un identifiant, pas une preuve d’origine, d’approbation ou de correction. Un budget de contenu entrant ou l’amplification causée par plusieurs champs différents ne sont pas évalués ici. Aucun LLM, AgentOS, Forgejo live, PR, merge, coût ou métrique de valeur n’est impliqué. Le projet demeure incomplet.
