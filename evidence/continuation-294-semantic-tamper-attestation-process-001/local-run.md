# CONTINUATION-294 — refus processus après falsification sémantique d’une ligne attestée

Date : 27 octobre 2026. Worktree : `/home/pc6/Documents/Codex/2026-10-21-mission-construire-une-forge-con-ue/.runtime/webhook-005-workspace`. HEAD observé : `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`; arbre préexistant très modifié, aucun commit créé.

## Question examinée

CONT-261 vérifiait que le reconciler exige une politique d’attestation avant de consommer le credential host. La matrice avait un contrôle unitaire pour une signature persistée modifiée et un contrôle processus pour une clé non fiable, mais il manquait un test processus combinant une clé fiable et un payload qui a été modifié après émission alors que le hash local non authentifiant est recalculé. Le lot vise uniquement ce cas.

## Scénario

Le test `actual_reconcile_cli_process_refuses_semantically_tampered_attestation_before_host_credential_read` démarre `forgejo-webhook-local` avec une graine Ed25519 de fixture, soumet via HTTP loopback une livraison `issues/opened` pour l’issue 41 et vérifie son acquittement. Après arrêt du receiver, il rejoue la ligne, change `payload.issue.number` en 42, conserve exactement `receiver_attestation.signature_hex` et recalcule `hash` sur la matière de hachage comprenant l’attestation persistée. Le journal accepte le replay de la chaîne; ceci démontre uniquement qu’un hash non signé peut être recalculé après altération.

Le test donne ensuite au vrai processus `forgejo-reconcile-local` la clé publique correspondant au receiver, un FD pipe portant un credential synthétique et l’ID de livraison. Une fixture API loopback fournit une réponse cohérente avec l’issue 42 si une requête est reçue, de sorte qu’un contrôle d’attestation supprimé ne puisse pas être confondu avec un échec de readback. Le test exige le refus `inconclusive/host_journal_attestation_invalid`, `host_credential_read=false`, `upstream_calls_attempted=false`, sortie non nulle, pipe credential byte-identical, zéro requête API observée et journal byte-identical.

## Résultat et vérifications

- Test isolé dans son namespace Bubblewrap : 1/1 réussi. Le runner impose `--unshare-net --unshare-pid`; l’accès externe n’est pas utilisé.
- `cargo test --locked --offline --all-targets` : **695 réussis, 0 échec, 155 ignorés, 37 suites**. Les tests ignorés, dont celui-ci, sont exclus de ce total.
- Le même scénario ignoré a été exécuté explicitement par `python3 scripts/run_webhook_reconcile_cli_e2e.py --scenario actual_reconcile_cli_process_refuses_semantically_tampered_attestation_before_host_credential_read` et passe 1/1.
- `cargo fmt --all -- --check`, `git diff --check`, le parse AST Python du runner et `python3 scripts/verify_cargo_license_state.py` réussissent.
- Le guard rapporte `license=null`, `license_file=null`, `publish=[]`, `no_license_selected`, `redistribution_rights=not_assessed`, `NOASSERTION`.
- Outils : rustc/cargo 1.95.0, Python 3.12.3, Bubblewrap 0.9.0, Linux/aarch64.

Le premier build du nouveau test a relevé une assertion Rust ambiguë (`Vec<String>` comparé à `[]`); elle a été remplacée par `requests.is_empty()`. Cet essai n’avait exécuté aucun scénario. Le log conservé est celui du build final et du test réussi.

## Fichiers concernés

Ajout test-only dans `tests/webhook_receiver_process_e2e.rs` et ajout du test à l’allowlist `scripts/run_webhook_reconcile_cli_e2e.py`; mise à jour de `docs/refusal-evidence-matrix.md` et de `docs/checkpoint.md`. Aucun runtime ni dépendance n’a changé. Les logs, sortie du guard, contrôle de format, parse Python et `git diff --check` sont dans ce dossier.

## Portée et limites

C’est une exécution Linux/aarch64 sur une livraison et des clés synthétiques (`n=1`), avec l’API remplacée par une fixture loopback; ce n’est pas une validation sur Forgejo live. Le résultat prouve que ce chemin CLI rejette cette signature Ed25519 après falsification du contenu, avant lecture du credential et avant appel upstream. Il ne prouve pas l’identité générale du writer du fichier, la vérification du HMAC d’origine au rejeu, l’authenticité de l’instance amont, la correction du code ou une politique de protection de clés en production. Aucun LLM/AgentOS, benchmark apparié, résultat de productivité ou gain général n’est démontré. La mission demeure incomplète.
