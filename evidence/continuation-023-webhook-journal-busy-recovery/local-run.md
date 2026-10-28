# CONTINUATION-023 — contention webhook actionable et reprise idempotente

Date : 26 octobre 2026. Produit `agent-forge-core`, worktree `/home/pc6/Documents/Codex/2026-10-21-mission-construire-une-forge-con-ue/.runtime/webhook-005-workspace`, branche `local/WEBHOOK-005-canonical-links`, HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. Le worktree était déjà fortement modifié; ses changements ont été préservés.

## Défaut reproduit

Le receiver traitait `JournalError::LockBusy` comme toute autre erreur de journal et répondait `503/journal_unavailable`. L’agent ne pouvait pas distinguer la contention momentanée d’une panne de stockage. Après le démarrage et le probe de readiness du vrai binaire, le test prend un `flock` exclusif sur le fichier latéral stable `.jsonl.lock`, puis envoie un webhook HMAC valide. Le test rouge `red-process-after-startup.log` observe le code générique au bout du délai de cinq secondes, là où il attend une réponse actionable `journal_busy`. Le test rouge de capacités `red-capabilities.log` établit séparément que l’endpoint ne documentait pas encore cette erreur.

Le premier essai `red-process.log` tenait le verrou avant le démarrage du receiver. Son replay de démarrage attendait le même verrou, empêchait la readiness et faisait échouer le harnais après cinq tentatives de port. Ce montage invalide n’établit aucun défaut produit; il est conservé pour rendre le diagnostic transparent.

## Changement et preuve verte

`src/webhook/http.rs` distingue maintenant `JournalError::LockBusy` et retourne `503/journal_busy`, `retryable=true`, action : `retry the exact signed delivery after the active inbox operation completes; do not edit the inbox`. L’endpoint `/forgejo/capabilities` déclare ce contrat sous `error_contract.journal_busy`.

Le test `actual_receiver_reports_retryable_journal_busy_and_accepts_same_delivery_after_release` traverse le vrai processus receiver : sous le verrou, la requête signée reçoit le nouveau refus et le contenu de l’inbox ne change pas; après libération, la même requête reçoit `202` avec `duplicate=false`; son rejeu reçoit `202` avec `duplicate=true`. Le replay final contient une livraison exactement. Le test des capacités passe aussi. Logs verts : `green-retry-e2e.log`, `green-capabilities.log`.

## Vérifications

- Tests processus ciblés : chacun passe 1/1; voir `green-retry-e2e.log` et `green-capabilities.log`.
- `cargo test --locked --offline --all-targets -- --test-threads=1` : 552 réussis, 0 échec, 76 ignorés, 34 groupes; résultat final dans `all-targets-final.log`. Le test de contention et le contrat des capacités figurent dans cette suite.
- `cargo fmt --all -- --check` et `git diff --check` passent; leurs sorties sont `rustfmt-final.log` et `diff-check-final.log`.
- `python3 scripts/verify_cargo_license_state.py` passe (`license-guard-final.log`). Un readback frais `cargo metadata --offline --locked --format-version 1 --no-deps` donne `license=null`, `license_file=null`, `publish=[]` (`cargo-metadata.json`). Aucun choix de licence ni conclusion sur les droits n’est fait.
- Toolchain et empreintes source : `tool-versions.txt`, `source-sha256.txt`. Les commandes complètes sont dans `commands.txt`.

## Limites

Le test utilise le vrai receiver local, un `flock` externe au processus receiver et un secret/payload fixture. Il démontre le comportement d’un processus sur cette machine, pas la contention distribuée, les garanties de `flock` sur tous les systèmes de fichiers, une livraison de Forgejo live ou la sécurité générale du journal. Aucun LLM, AgentOS, secret utilisateur, coût, service externe, instance de production, publication ou code Forgejo n’intervient. Aucun commit n’est créé. Le projet et le parcours LLM restent incomplets.
