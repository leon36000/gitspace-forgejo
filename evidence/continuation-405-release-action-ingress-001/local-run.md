# CONTINUATION-405 — release action au processus receiver et accès GitLab

Date : 2026-10-28. Worktree : `local/WEBHOOK-005-canonical-links`, HEAD observé `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. Aucun `fetch`, `ls-remote`, authentification, staging, commit ou push GitLab n’a été tenté.

## Scénario du receiver

Ajout d’un test processus isolé dans `tests/webhook_receiver_process_e2e.rs`, déclaré dans `scripts/run_webhook_reconcile_cli_e2e.py`. Le binaire HTTP receiver reçoit un corps JSON signé par HMAC synthétique sur loopback. L’action Forgejo `release.created` est refusée en HTTP 400 avec `event_payload_invalid`, `retryable=false` et `details.field_path=payload.action`; la relecture de l’inbox est vide. Le même delivery ID est ensuite accepté après correction en `published` (sequence 1). Des livraisons `updated` et `deleted` distinctes sont acceptées en sequences 2 et 3. La relecture finale contient exactement ces trois actions sous le kind `forgejo.webhook.v1.release`.

Commande : runner `run_webhook_reconcile_cli_e2e.py --scenario actual_receiver_rejects_invalid_release_action_then_accepts_corrected_same_delivery`, avec son répertoire temporaire redirigé vers `/dev/shm` parce que `/` était à 100% et n’avait que 121 MiB disponibles. La compilation et le test s’exécutent dans Bubblewrap 0.9.0 avec `--unshare-net --unshare-pid`; le résultat ciblé est **1 passed, 0 failed, 193 filtered out**, code de sortie 0. Journal brut : `process-test.log`. L’usage de tmpfs évite de supprimer ou déplacer les fichiers préexistants.

Le test ne modifie pas la logique runtime : le changement porte sur le test et son allowlist de runner, qui sont tous deux non suivis par Git dans l’état actuel du worktree. `cargo fmt --all -- --check`, `git diff --check` (fichiers suivis), et le parse AST Python passent. La garde Cargo passe 12/12; `cargo metadata --offline --locked --format-version 1 --no-deps` confirme `license=null`, `license_file=null`, `publish=[]`, `decision=no_license_selected`, `redistribution_rights=not_assessed`, `spdx_package_license_declared=NOASSERTION`.

## Portée et limites

Cette preuve démontre le contrat d’action au processus HTTP et le non-enregistrement/retry du même ID avec un événement signé par une fixture. Elle ne montre pas que Forgejo natif a émis le webhook; ne couvre pas à nouveau la lecture API release déjà prouvée séparément; ne démontre ni LLM/AgentOS, PR, merge, isolation multi-tenant ni gain comparatif. Le test ne modifie ni Forgejo ni le service de vérification. Aucune suite complète n’a été relancée dans ce lot.

## Prévol GitLab PC4, sans authentification

Le courriel « Accès GitLab et SonarQube sur PC4 » (27 octobre 2026) précise que les outils PC4 sont accessibles via Tailscale. Le courriel distinct de jeton confirme un rôle Développeur et un jeton limités aux projets Nexora WorkStudio et Nexora Web; ce périmètre ne comprend pas `root/gitspace-forgejo`. Le courriel d’initialisation contient aussi un mot de passe initial pour le compte `root`. Aucun secret, corps de courriel, identifiant de message ou valeur de credential n’est conservé ici. Le mot de passe administrateur n’a pas été utilisé et le jeton hors périmètre n’a pas été essayé.

Les reçus antérieurs établissent que le FQDN PC4 ne se résout pas depuis PC6 et que TCP/443 vers les adresses Tailscale observées expire avant HTTP. Aucun changement de routage n’ayant été signalé, ce test réseau n’a pas été rejoué. `origin` reste le Forgejo local et `pc4-gitlab` reste `https://pc4.tail0c55ff.ts.net/root/gitspace-forgejo.git` pour fetch/push; aucune référence GitLab n’a été lue. Le transfert attend une route PC6→PC4 HTTPS fonctionnelle et un credential `write_repository` limité au dépôt demandé.

## Références des vérifications

- `process-test.log` — compilation offline sous Bubblewrap et résultat ciblé 1/1.
- `cargo-license-guard-tests.log`, `cargo-license-guard.json`, `cargo-metadata-license-summary.json` — politique de licence Cargo inchangée, aucune licence choisie.
- `rustfmt.log`, `diff-check-tracked.log`, `runner-ast-check.log` — checks locaux passés.
- `environment.txt` — versions des outils, branche, HEAD, remotes et espace disque.
- `source-hashes.sha256` — hashes des sources et journaux de ce lot.
