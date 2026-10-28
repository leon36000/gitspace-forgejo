# CONTINUATION-416 — refus base PR actionnable et retry corrigé

Date : 28 octobre 2026. HEAD de base observé : `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`; branche `local/WEBHOOK-005-canonical-links`. Le worktree préexistant est fortement modifié. Aucun état n’a été réinitialisé, commité ou envoyé à distance.

## Portée du changement

La vérification que `pull_request.base.repo` correspond au dépôt racine et qu’un `head.repo` peut être un fork est antérieure à ce lot et déjà attestée par [WEBHOOK-PR-BASE-REPOSITORY-IDENTITY-PROCESS-001](../webhook-pr-base-repository-identity-process-001/local-run.md). CONT-416 ne la revendique pas comme nouvelle capacité; il corrige le diagnostic ingress et éprouve les retries.

Dans `src/webhook/reconcile.rs:2370-2379`, les erreurs de dépôt base mal formé ou contradictoire sont ramenées au champ `pull_request.base.repo`; une enveloppe `pull_request.base` non objet est ramenée à `pull_request.base`. `src/webhook/http.rs:383-387` fournit une action propre à chacun : corriger le payload auprès du sender de confiance, garder l’identité de base égale à `repository.full_name`, laisser un fork contributeur sur `head.repo`, consulter les capacités et redélivrer sous le même delivery ID. Le corps rejeté ne fait pas partie de la réponse.

## Scénario processus

`tests/webhook_receiver_process_e2e.rs:1870-2127::actual_receiver_binds_pull_request_base_repository_but_allows_fork_head` lance le vrai binaire receiver et envoie des payloads HMAC de fixture par HTTP loopback :

1. Une PR avec dépôt base configuré et dépôt `head.repo` d’un contributeur est admise à la séquence 1.
2. Une base étrangère est refusée en HTTP 400 avec `payload.pull_request.base.repo`, conseil actionnable, absence du marqueur privé et journal octet-identique. La correction signée sous le même ID est admise à la séquence 2.
3. `pull_request.base` de type chaîne est refusé avec `payload.pull_request.base`, sans append ni fuite; sa correction sous le même ID est admise à la séquence 3.
4. `pull_request.base.repo.full_name` numérique est refusé avec `payload.pull_request.base.repo`, sans append ni fuite; sa correction sous le même ID est admise à la séquence 4.

Le replay final contient quatre lignes : l’admission initiale et les trois corrections; aucun payload rejeté n’est journalisé.

Le test ciblé a d’abord échoué avant le mapping runtime parce que `field_path` était `null`; ce run rouge est rapporté par le résumé de session mais sa sortie brute n’a pas été conservée dans le dossier de preuve. Le reçu reproductible et complet est le run vert ci-dessous. La première vérification rustfmt a aussi échoué sur deux indentations du nouveau test; elles ont été formatées puis le contrôle final est passé.

## Vérifications fraîches

- Processus exact : `cargo test --offline --locked --test webhook_receiver_process_e2e actual_receiver_binds_pull_request_base_repository_but_allows_fork_head -- --exact --test-threads=1` — **1 réussi, 0 échec, 202 filtrés**. Sortie : `focused-process-final.log`.
- Contrats : `cargo test --offline --locked --test webhook_contract --test webhook_handler_contract --test webhook_http_contract --test webhook_reconcile_contract -- --test-threads=1` — **147 réussis, 0 échec** (10 + 30 + 7 + 100). Sortie : `contracts-final.log`.
- `rustfmt --edition 2024 --check src/webhook/reconcile.rs src/webhook/http.rs tests/webhook_receiver_process_e2e.rs` — réussi (`rustfmt-final.log`).
- `git diff --check` sur le diff suivi — réussi (`diff-check-final.log`); `rustfmt --check` couvre aussi le test non suivi.
- `python3 scripts/verify_cargo_license_state.py` — réussi; `cargo metadata --offline --locked --no-deps --format-version 1` rapporte package `agent-forge-core` 0.1.0, `license=null`, `license_file=null`, `publish=[]`. Décision inchangée : `no_license_selected`, droits de redistribution `not_assessed`, SPDX `NOASSERTION` (`cargo-license-guard-final.json`, `cargo-metadata.json`).

Toolchain et empreintes des trois sources pertinentes : `environment.txt`, `source-hashes.sha256`. Cargo a utilisé `CARGO_TARGET_DIR=/dev/shm` et les dépendances verrouillées hors ligne.

## Isolement et limites

Deux essais d’exécution de ce binaire sous Bubblewrap n’ont pas lancé le test : le binaire `/dev/shm/debug/deps/...` n’était pas visible dans le namespace (sorties conservées dans `isolated-process*.log`). Le succès n’est donc **pas** présenté comme une preuve de confinement réseau/PID. Le test réussi est local et ne contacte que le receiver loopback qu’il démarre; il ne teste aucun agent exécutant du code non fiable.

Pas de webhook Forgejo live, instance Forgejo, modèle LLM, AgentOS, RAG ou service mémoire dans ce lot. Pas de run `all-targets` et aucune comparaison de qualité, durée, tokens ou coût. Ce reçu ne clôt aucun critère global de la mission.
