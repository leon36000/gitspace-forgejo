# CONTINUATION-191 — réponses Forgejo trop grandes : refus explicite et diagnostic

Date : 27 octobre 2026. Worktree : `/home/pc6/Documents/Codex/2026-10-21-mission-construire-une-forge-con-ue/.runtime/webhook-005-workspace`. HEAD de base : `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`; arbre fortement modifié. Aucun commit n’a été créé.

## Changement et constat

Le client avait déjà des limites de taille pour le corps HTTP, le JSON d’en-têtes curl et la sortie capturée d’un enfant, mais les erreurs apparaissaient comme `upstream_invalid_response`. Le lot introduit `ApiReadError::ResponseTooLarge { component, maximum_bytes }`. Les réponses trop grandes produisent maintenant `upstream_response_too_large`, non réessayable, avec une action de correction et des détails bornés. Le CLI annonce cette capacité et les composants connus (`body`, `header_json`, `captured_output`) dans schema 4 / `forgejo-reconcile-capabilities-v4`.

Le test d’intégration processus `actual_reconcile_cli_process_refuses_oversized_current_api_header_json` couvre le receiver signé, son journal et le CLI réel. Une fixture loopback fournit un JSON d’issue valide avec un en-tête synthétique de 70 KiB contre une limite de 65 536 octets. Sous Bubblewrap (`--unshare-net --unshare-pid`), le CLI termine avec code 1 et verdict `inconclusive`; le code d’erreur est `upstream_response_too_large`, `retryable=false`, `details.component=header_json` et `details.maximum_bytes=65536`. Il n’émet aucun état canonique ni marqueur d’en-tête, sa sortie reste sous 16 KiB, la fixture observe exactement un GET sans Authorization, et le journal reste identique octet pour octet.

Un échec rouge du test processus prouvait que le comportement précédent ne respectait pas le diagnostic attendu (`upstream_invalid_response`). Après l’implémentation, le même processus isolé passe 1/1. Le chemin runtime de prévalidation merge avait le code d’erreur correct mais remplaçait l’action et les détails par `{}`. Le test unitaire `final_merge_state_runtime_error_preserves_oversized_response_details` échoue d’abord sur l’action absente, puis passe après correction et vérifie le code, le non-retry, l’action, `upstream`, le composant et la limite.

Le scénario `actual_reconcile_cli_process_refuses_unbounded_chunked_current_api_response` a aussi été relancé : fixture HTTP chunked de 2 MiB sans `Content-Length`, budget corps de 1 MiB, refus `upstream_response_too_large` avec composant `body` ou `captured_output` et limite positive; pas d’état canonique/partiel, une requête sans Authorization, inbox inchangée et stdout <16 KiB. Résultat Bubblewrap : 1/1.

## Vérifications reproductibles

- `cargo test --offline --locked --bin forgejo-reconcile-local final_merge_state_runtime_error_preserves_oversized_response_details -- --nocapture` — 1 réussi.
- `python3 scripts/run_webhook_reconcile_cli_e2e.py --scenario actual_reconcile_cli_process_refuses_oversized_current_api_header_json` — processus isolé, 1/1.
- `python3 scripts/run_webhook_reconcile_cli_e2e.py --scenario actual_reconcile_cli_process_refuses_unbounded_chunked_current_api_response` — processus isolé, 1/1.
- `cargo test --offline --locked --all-targets` — 659 réussis, 0 échec, 124 ignorés, 36 cibles. Les deux tests processus `#[ignore]` ont été exécutés séparément comme ci-dessus.
- `cargo fmt --all -- --check`, `git diff --check`, parsing AST du lanceur Python — passent.
- `cargo metadata --offline --locked --no-deps --format-version 1` et `python3 scripts/verify_cargo_license_state.py` — `license=null`, `license_file=null`, `publish=[]`, `no_license_selected`, droits de redistribution `not_assessed`, SPDX déclaré `NOASSERTION`.

Sorties correspondantes : `runtime-diagnostic-red.log`, `runtime-diagnostic-green.log`, `process-red.log`, `process-green.log`, `process-header-final.log`, `process-chunked-final.log`, `chunked-process-green.log`, `capabilities-red.log`, `capabilities-green.log`, `capabilities-version-red.log`, `capabilities-version-green.log`, `all-targets-final.log`, `rustfmt-final.log`, `diff-check-final.log`, `runner-ast-final.log`, `cargo-metadata.json` et `cargo-license-state.json`. `toolchain.txt` consigne rustc 1.95.0, Cargo 1.95.0, Python 3.12.3, Bubblewrap 0.9.0 et curl 8.5.0.

## Portée et limites

Les fixtures HTTP sont synthétiques et chaque scénario n’est exécuté qu’une fois dans ce reçu. Le test ne localise pas précisément la couche qui a atteint la limite entre curl, la conversion de ses en-têtes et le parseur. Il ne teste pas Forgejo vivant, les versions/configurations externes, la consommation exacte d’octets avant annulation, la charge soutenue ou la production. L’ajout de diagnostic n’est pas une preuve de correction de la réponse ni une garantie de sécurité générale. Aucun agent LLM, AgentOS, RAG ou service externe n’est impliqué. Aucune licence de produit n’a été choisie.
