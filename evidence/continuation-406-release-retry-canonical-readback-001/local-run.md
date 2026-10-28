# CONTINUATION-406 — retry Release corrigé puis readback courant

Date : 2026-10-28. Worktree : `local/WEBHOOK-005-canonical-links`, HEAD de base observé `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`.

## Parcours et résultat

Le scénario processus existant `actual_reconcile_cli_process_reads_current_release_by_id_and_omits_notes` couvre désormais en séquence le refus ingress, le retry et le readback par le vrai CLI. Le receiver reçoit un HMAC synthétique valide avec `action=created`; il renvoie HTTP 400 `event_payload_invalid`, `retryable=false`, `details.field_path=payload.action`, et l’inbox reste vide. Il reçoit ensuite `action=updated` avec le même delivery ID et acquitte la sequence 1. Le receiver scelle le journal avec une clé de test; le processus réel `forgejo-reconcile-local` vérifie cette attestation et effectue un unique GET anonyme vers `/api/v1/repos/{owner}/{repo}/releases/610`.

La fixture API répond avec le tag courant `v1.4.2`, distinct de `stale-tag` dans le webhook. Le résultat est `current_state_read`, `upstream_read_attempts=1`, ressource Release 610, attestation `verified` avec portée `receiver_signature_only`; la projection vient du readback courant. Les notes et assets de l’événement et de la fixture ne sont pas inclus, les marqueurs privés sont absents de stdout/stderr et le journal reste octet-identique après la lecture.

Exécution : `run_webhook_reconcile_cli_e2e.py --scenario actual_reconcile_cli_process_reads_current_release_by_id_and_omits_notes`. Le répertoire temporaire du runner a été redirigé vers `/dev/shm` parce que la partition racine manque d’espace. La compilation Cargo est offline; Bubblewrap 0.9.0 lance le test avec `--unshare-net --unshare-pid`, tandis que la fixture HTTP et les processus testés partagent leur réseau privé. Résultat : **1 passed, 0 failed, 193 filtered out**, code de sortie 0. Voir le log brut `process-test.log`.

Contrôles : `cargo fmt --all -- --check`, `git diff --check` pour les fichiers suivis, vérification AST du runner et contrôle d’espaces de fin Rust passent. `verify_cargo_license_state.py` émet `metadata_policy_verified`; Cargo metadata donne `license=null`, `license_file=null`, `publish=[]`, décision `no_license_selected`, droits `not_assessed`, marqueur `NOASSERTION`. Aucun champ de licence n’est sélectionné. Les versions, source hashes et checksums sont dans `environment.txt` et `source-hashes.sha256`.

## Portée et limites

Ce run produit un log reproductible pour le readback Release processus, là où le run antérieur de CONTINUATION-404 n’avait laissé qu’un résultat rapporté sans log. Il démontre receiver → refus sans réservation → retry corrigé → journal scellé localement → readback CLI ciblé. La signature, le journal, le serveur API et la réponse courante sont des fixtures locales; aucun Forgejo vivant ni webhook natif n’est utilisé. L’attestation prouve uniquement la signature sous la clé locale de test, pas la vérité du payload ou du serveur. Aucun LLM/AgentOS, PR, décision de merge, fusion, sécurité multi-tenant ou comparaison de valeur n’est démontré. Aucune logique runtime n’a changé et aucune suite complète n’a été relancée.
