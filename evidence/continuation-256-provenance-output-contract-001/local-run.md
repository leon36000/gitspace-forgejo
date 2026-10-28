# CONTINUATION-256 — assertion processus exacte du contrat de provenance

Date : 27 octobre 2026. Worktree `/home/pc6/Documents/Codex/2026-10-21-mission-construire-une-forge-con-ue/.runtime/webhook-005-workspace`, HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. Aucun code de production n’a changé.

## Lacune et correction

Après CONT-253, les sorties single v11 et batch v12 ont un objet `source_event_provenance_contract` de neuf champs. Le helper partagé des tests processus n’assertait que `replay_hmac_verification` et `source_authenticity_claim_for_selected_delivery`; une régression supprimant ou inversant les sept autres valeurs pouvait donc rester verte.

TDD rouge : `reconcile_process_contract_assertion_rejects_incomplete_provenance` lui fournit un résultat v11 contenant uniquement ces deux champs. Avant la correction, le helper l’acceptait et le nouveau test échoue comme prévu (voir `red.log`). Le helper compare maintenant l’objet complet, y compris les faits négatifs : aucune attestation par livraison, signature brute non persistée, writer local non authentifié par la chaîne, frontière `local_filesystem_writer`, et `current_state_read` limité à une lecture API canonique. Le test de contrat refuse désormais un objet incomplet.

## Vérifications après correction

- `cargo test --offline --locked --test webhook_receiver_process_e2e` : 37 réussis, 0 échec, 118 ignorés.
- Sous le runner Bubblewrap (`--unshare-net --unshare-pid --new-session`, `prlimit`), scénario single `actual_reconcile_cli_process_reads_gitea_alias_only_delivery` : 1/1, 154 filtrés. Le vrai receiver accepte une livraison HMAC synthétique, le vrai CLI lit une issue sur fixture HTTP, le test vérifie schema v11 et l’objet exact.
- Sous le même runner, batch `actual_reconcile_cli_process_reads_receiver_journal_against_private_http_fixture` : 1/1, 154 filtrés; le contrat exact commun est vérifié sur le résultat batch.
- `cargo fmt --all -- --check`, `git diff --check` et `py_compile` des scripts concernés passent. Bubblewrap 0.9.0; Rust/Cargo 1.95.0.

Logs complets, commande de rejeu, résumés, versions, hashes de 11 sources et manifestes append-only sont dans ce dossier.

## Limites

La nouvelle assertion renforce le harnais de test, pas le runtime. Les deux scénarios utilisent une API loopback synthétique et aucun serveur Forgejo, credential réel, LLM ou AgentOS. Le single s’appuie sur les alias `X-Gitea-*` acceptés par le récepteur; le batch utilise les en-têtes Forgejo du helper. Le journal demeure non authentifié contre un writer local capable de le réécrire et de recalculer la chaîne. Aucun pull request/merge ou gain comparatif n’est démontré. Le test cible exact du schéma doit être mis à jour avec toute évolution intentionnelle de ces champs.
