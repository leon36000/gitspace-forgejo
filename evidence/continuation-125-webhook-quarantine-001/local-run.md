# CONTINUATION-125 — quarantaine append-only d’une livraison invalide déjà persistée

Date : 26 octobre 2026. Worktree : `/home/pc6/Documents/Codex/2026-10-21-mission-construire-une-forge-con-ue/.runtime/webhook-005-workspace`. HEAD de base : `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`; branche `local/WEBHOOK-005-canonical-links`; checkout fortement modifié et non commité. Les changements de cette tranche sont locaux; aucun service Forgejo, modèle, service externe ou environnement de production n’a été lancé.

## Voie ajoutée

Une livraison webhook hash-chain-valid qui devient structurellement invalide au replay ne pouvait jusque-là être ni traitée ni mise de côté sans toucher au journal. Le CLI trusted accepte maintenant :

```sh
forgejo-reconcile-local --quarantine-invalid-delivery <inbox.jsonl> <delivery-id> <source-sequence> <source-sha256>
```

Sous le verrou du journal, la commande relit le journal sans réparer une queue partielle et exige que la séquence, le delivery ID et le hash désignent encore exactement une livraison d’un type supporté. Une erreur de validation appartenant à une liste fermée est recalculée depuis la ligne source. Une livraison valide, un hash ou une séquence périmé, un type non supporté, un journal corrompu ou une queue partielle ne peut pas être mis en quarantaine. Le type `forgejo.webhook.quarantine.v1` est réservé : l’API générique `EventJournal::append` ne peut pas le forger.

La disposition ajoutée contient le delivery ID, la séquence, l’event ID et le hash de la source ainsi que le code de validation dérivé. Sa clé d’idempotence rend le même choix répétable. Les lecteurs revérifient ces liens; une source dont le hash ne correspond plus rend la disposition invalide. La ligne source n’est ni modifiée ni supprimée. La réconciliation retourne `inconclusive/delivery_quarantined`, sans état canonique. Elle saute cette livraison avant de lire le credential host; les lots ne réconcilient que leurs autres livraisons non quarantinées. L’admission de mission reçoit également une erreur explicite.

Le sous-commandement est une capacité d’opérateur sur l’exécutable host existant; aucun nouvel outil MCP ou permission d’agent n’est ajouté. La revue statique de `src/bin/mission-012-mcp-host.rs` confirme que `model_tools()` n’expose que `mission_context`, `workspace_read_text`, `workspace_write_text` et `cargo_test` (vers les lignes 1684–1724), et qu’aucune commande CLI de réconciliation n’est dispatchée par `tools/call`. L’inbox ne persiste pas le HMAC de réception. La commande ne l’a donc ni vérifié à nouveau, ni enregistré l’identité humaine de l’opérateur. Le hash du journal prouve seulement l’intégrité de la ligne courante. Le résultat annonce explicitement `source_hmac_persisted=false`, `source_hmac_reverified=false`, `actor_identity_recorded=false` et `canonical_state_created=false`. Cette tranche ne réimporte pas un webhook, ne répare pas le source et ne restaure aucun état canonique.

## Vérifications

- `quarantine_is_bound_idempotent_and_never_reconciles_invalid_source` passe : source inchangée, rejet de `EventJournal::append` générique, rejet du hash périmé, raison `payload.action`, état `QuarantinedDelivery`, disposition idempotente. Le test change aussi le hash dans un snapshot mémoire et vérifie que le lecteur de disposition refuse la référence.
- `quarantine_refuses_a_torn_journal_tail_without_repairing_it` passe : l’opération renvoie `JournalError::Corrupt` et le fichier reste byte pour byte identique.
- `trusted_quarantine_command_appends_once_and_reconcile_refuses_before_reads_or_credential` passe : le chemin d’exécution interne refuse toute lecture amont et ne consomme pas le pipe de credential.
- Le test d’intégration `real_cli_quarantines_only_the_exact_invalid_row_and_never_reports_success` passe avec le binaire Cargo réel : hash périmé refusé, ligne invalide quarantinée, répétition idempotente, source conservée, ligne valide refusée, puis replay CLI non nul avec `inconclusive/delivery_quarantined`, compteur interne `upstream_read_attempts=0` et sans état canonique.
- `cargo test --offline --locked --all-targets` : 619 réussis, 0 échec, 106 ignorés, sur 36 exécutables de test. Les tests ignorés n’ont pas été exécutés par ce run.
- `cargo check --offline --locked --all-targets`, `cargo fmt --all -- --check` et `git diff --check` finaux réussissent. Un premier contrôle rustfmt a trouvé uniquement un retour de ligne; `cargo fmt` l’a corrigé avant ces contrôles finaux.
- `cargo metadata --offline --locked --no-deps --format-version 1` confirme `license=null`, `license_file=null`, `publish=[]`, `no_license_selected`, droits `not_assessed`, SPDX `NOASSERTION`.

Les sorties de tests, compteurs, métadonnées, empreintes et toolchain se trouvent dans ce dossier. Versions observées : `rustc 1.95.0 (59807616e 2026-04-14)`, Cargo `1.95.0 (f2d3ce0bd 2026-03-21)`, Python `3.12.3`.

## Portée et limites

Les fixtures de cette tranche sont synthétiques et ne portent pas de HMAC vérifiable; le test d’intégration lance le vrai CLI mais pas une instance Forgejo ni un webhook émis par celle-ci. Le compteur zéro est celui du CLI; aucune capture réseau externe n’a été utilisée. Le changement de hash après disposition est vérifié sur un snapshot mémoire, pas par réécriture/rechaînage du fichier sur disque. La commande suppose que son appelant host est trusted; aucun identifiant d’opérateur n’est enregistré. Une queue ou chaîne corrompue reste refusée et requiert une procédure de restauration séparée. La licence GitSpace demeure indéterminée; AgentOS/RAG/mémoire et l’intégration LLM ne sont pas démontrés ici. Le benchmark contrôlé et l’achèvement global restent ouverts.

Le run est lié aux fichiers et journaux listés dans `source-hashes.txt` et protégé par le manifeste local append-only; la dernière vérification est `verify_append_only_indexes.py`.
