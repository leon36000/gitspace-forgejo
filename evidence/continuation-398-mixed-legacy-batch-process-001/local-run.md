# CONTINUATION-398 — batch mixte legacy au CLI processus réel

Date : 28 octobre 2026. Worktree local `/home/pc6/Documents/Codex/2026-10-21-mission-construire-une-forge-con-ue/.runtime/webhook-005-workspace`, branche `local/WEBHOOK-005-canonical-links`, HEAD de base `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. Le dépôt était déjà sale; aucun changement antérieur n’a été réinitialisé, stage ou commité.

## But et protocole

CONT-397 vérifiait qu’un tag delete historique ne bloquait pas une branche valide dans `reconcile_inbox_events`, avec `FixtureApi` en mémoire. CONT-398 ajoute la couche processus CLI : un helper de test écrit deux lignes webhook legacy synthétiques dans une inbox mode 0600, avec séquences, identités, clés idempotentes et hash chain valides; `EventJournal::replay()` relit les deux lignes avant le run. L’événement tag n’est pas envoyé au receiver courant, qui le refuserait justement à l’ingress.

Le runner démarre ensuite le vrai `forgejo-reconcile-local` comme sous-processus Bubblewrap avec namespaces PID/réseau isolés, plus une HTTP fixture locale sur `127.0.0.1:3130`. Pour le batch `[tag delete, branch delete]` :

- Le tag renvoie un résultat per-delivery `inconclusive`, erreur `unsupported_event`, chemin `payload.ref_type`, sans état canonique.
- Le CLI continue la branche et renvoie `current_state_read`, état `present`, head SHA `cccc…cccc`.
- Une seule requête HTTP est observée : GET du readback courant de la branche. Aucun header Authorization n’est envoyé.
- La comparaison byte-for-byte confirme que le CLI ne réécrit pas le journal legacy.

## Commande et résultat

```text
python3 scripts/run_webhook_reconcile_cli_e2e.py --scenario actual_reconcile_cli_process_continues_branch_batch_with_legacy_tag_delete
```

Run final dans `mixed-legacy-batch-process-final.log` : 1 passé, 0 échec, 0 ignoré, 189 filtrés; le premier run est conservé dans `mixed-legacy-batch-process.log`. Environnement exact du runner : Bubblewrap `--unshare-net --unshare-pid`, Cargo `--offline --locked`, HTTP fixture loopback. L’inbox repose sur un helper de test synthétique; aucun receiver, réseau externe ou Forgejo live n’est appelé.

`cargo fmt --all -- --check`, `git diff --check` et `python3 scripts/verify_cargo_license_state.py` passent (`rustfmt-check-final.log`, `diff-check-final.log`, `license-state-final.log`). Le premier contrôle rustfmt ne signalait qu’un retour à la ligne; après correction, le scénario est relancé sur le source final. Garde Cargo : `license=null`, `license_file=null`, `publish=[]`, `license_decision=no_license_selected`, `redistribution_rights=not_assessed`, SPDX `NOASSERTION`.

Outils : `rustc 1.95.0 (59807616e 2026-04-14)`, `cargo 1.95.0 (f2d3ce0bd 2026-03-21)`, Bubblewrap 0.9.0.

## Identité des sources

Hashes de fichiers entiers du worktree sale après le lot, pas de patches isolés :

```text
7d7fc5300cc7889bed925cf52fd5e821cd9c7376807207a0e150d7b13993fe29  tests/webhook_receiver_process_e2e.rs
b928f9af1b94b835929cf0cd34f9298e9d4ccc7670f5026c456bb9265d5b1162  scripts/run_webhook_reconcile_cli_e2e.py
```

Les manifestes append-only locaux, worktree et projet lient rapport, logs et documents, tout en préservant les préfixes : `SHA256SUMS`, `manifest-prefix-before.json`, `manifest-check.txt`, `global-manifest-check.txt`.

## Limites

Le journal simulé n’est pas produit par une ancienne release et les sources ne sont pas émises par Forgejo. Ce test couvre le processus du CLI et la poursuite d’un batch, pas l’intégration receiver→CLI réelle, l’authentification Forgejo, le rétablissement après crash, ni concurrence. Aucun code d’exécution ne change; suite all-targets non rejouée. Aucun AgentOS/LLM, service externe, réseau, dépense ou production. La mission reste incomplète.
