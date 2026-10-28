# CONTINUATION-024 — `Retry-After` pour `journal_busy`

Date : 26 octobre 2026. Produit `agent-forge-core`, worktree `/home/pc6/Documents/Codex/2026-10-21-mission-construire-une-forge-con-ue/.runtime/webhook-005-workspace`, branche `local/WEBHOOK-005-canonical-links`, HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. Le dépôt avait déjà de nombreuses modifications; elles sont conservées.

## Constat et changement

Après CONTINUATION-023, le receiver renvoyait un `503/journal_busy` réessayable et une action lisible, mais n’offrait pas de délai HTTP aux émetteurs. Le test de processus a d’abord échoué : `Retry-After` était absent (`red-process.log`).

`HttpResponse` porte maintenant un `retry_after_seconds` optionnel. Pour `JournalError::LockBusy`, le handler le fixe à `ceil(EVENT_JOURNAL_LOCK_WAIT_MS / 1000)`, soit 5 secondes avec la configuration actuelle. Le binaire écrit `Retry-After: 5` uniquement lorsque la réponse inclut cette valeur. L’endpoint `/forgejo/capabilities` annonce le même `retry_after_seconds: 5` pour `journal_busy`. Les réponses d’acceptation `202`, dont le doublon idempotent, n’ont pas cet en-tête.

## Vérification

`actual_receiver_reports_retryable_journal_busy_and_accepts_same_delivery_after_release` démarre le vrai binaire receiver, tient son fichier `.jsonl.lock` avec `flock`, puis envoie le même webhook HMAC fixture. La réponse occupée est `503`, porte `Retry-After: 5` et n’ajoute rien au journal. Après libération, le corps signé identique est accepté en `202` sans `Retry-After`; son nouvel envoi est `202/duplicate=true`, également sans l’en-tête. Le replay final contient une seule livraison. Le test du contrat de capacités vérifie la valeur annoncée.

- Les deux tests ciblés passent 1/1; logs `green-process.log` et `green-capabilities.log`.
- `cargo test --locked --offline --all-targets -- --test-threads=1` : 552 réussis, 0 échec, 76 ignorés, 34 groupes (`all-targets.log`). Les deux tests sont présents dans ce run.
- `cargo fmt --all -- --check`, `git diff --check` et `python3 scripts/verify_cargo_license_state.py` passent (`rustfmt-check.log`, `diff-check.log`, `license-guard.log`).
- Readback `cargo metadata --offline --locked --format-version 1 --no-deps` : `license=null`, `license_file=null`, `publish=[]` (`cargo-metadata.json`). Le produit conserve `no_license_selected`; aucun droit de redistribution n’est inféré.
- Commandes, toolchain et hashes des sources figurent dans `commands.txt`, `tool-versions.txt` et `source-sha256.txt`.

## Portée et limites

L’en-tête communique un délai fixe égal au timeout du verrou local; il ne mesure pas la durée restante de l’opération qui détient le verrou et ne garantit pas que la prochaine tentative réussira. Le test prouve le comportement du processus receiver local sur ce système, avec un verrou et une livraison fixture; il ne prouve pas Forgejo live, un stockage distribué ou le comportement d’autres clients webhook. Aucun LLM, AgentOS, donnée privée, coût, publication, production ou code Forgejo n’intervient. Le projet reste incomplet.
