# CONTINUATION-374 — fixer les pins du vérificateur dans le host local

Date : 28 octobre 2026. Worktree `local/WEBHOOK-005-canonical-links`, HEAD de base `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. Le dépôt est fortement modifié; ce lot conserve les changements existants et ne crée ni commit ni push. L’état et les versions exactes sont consignés dans `environment.txt`; les sources du lot sont liées dans `source-sha256.json`.

## Changement

Le binaire `mission-012-verify-local` acceptait deux arguments de digest, puis `verify_pair` les comparait aux valeurs compilées `EXPECTED_REQUIREMENT_SHA256` et `EXPECTED_POLICY_SHA256`. Un mauvais digest ne pouvait pas obtenir `pass`, mais la CLI exposait un faux choix d’autorité et créait le répertoire cible avant son refus.

La nouvelle interface retire ces deux arguments. Le build du vérificateur fixe les digests et `/toolchain/bin/cargo test --offline --locked`; les fichiers de policy du snapshot doivent correspondre au digest compilé et déclarer cette commande. Le host appelant ne peut donc pas sélectionner d’autres critères ou une commande de vérification par les arguments de cette CLI.

Un test de processus modifie la policy de base et candidate pour demander `/tmp/attacker-controlled-command`, puis met aussi à jour `policy/active.json` vers le nouveau digest. Le vérificateur répond `inconclusive/requirement_or_policy_binding_invalid_or_stale`; il n’exécute pas cette commande. Le test positif lit les commits et snapshots d’un dépôt Git de fixture, observe le test protégé rouge sur la base, puis vert sur le candidat et lie le rapport aux empreintes/à la commande compilées.

## Vérifications

- Test-first sur l’ancienne CLI : `scripts/run_forgejo_mission_delivery_process_e2e.py --only verifier_process_refuses_caller_supplied_requirement_and_command` échoue sur la nouvelle attente exit 64. Le binaire ancien avait accepté la forme avec digests et répondu exit 2 `inconclusive/verification_binding_arguments_invalid`; aucun succès n’a été obtenu. La cible avait été créée, ce qui motivait la réduction d’interface. Log : `test-first-red.log`.
- Après changement, la même commande passe 1/1 sous Bubblewrap `--unshare-net --unshare-pid`; la forme avec digests et `--verification-command` sont refusés avant la création de leurs cibles : `verifier-process-green.log`.
- `cargo test --offline --locked --bin mission-012-verify-local verifier_rejects_a_candidate_policy_that_names_another_command -- --nocapture` passe 1/1 après correction de fixture. Le premier essai avait deux commits identiques car le candidat ne changeait pas; ce défaut du test est conservé dans `policy-command-tamper-targeted.log`, et l’essai correct est dans `policy-command-tamper-targeted-final.log`.
- `cargo test --offline --locked --bin mission-012-verify-local independent_pair_verifier_binds_fresh_red_green_to_exact_snapshots -- --nocapture` passe 1/1 et vérifie aussi `report.command`: `verifier-command-binding-unit.log`.
- `cargo test --offline --locked --test forgejo_mission_delivery_process_e2e verifier_cli_process_runs_pinned_red_green_without_caller_digests -- --exact --ignored --nocapture` passe 1/1. Ce probe sans socket appelle le vrai binaire; son candidat est exécuté uniquement dans la sandbox interne Bubblewrap/systemd, hors ligne : `verifier-cli-process-red-green.log`.
- `cargo fmt --all -- --check`, `git diff --check`, l’analyse syntaxique Python, les 12 tests de `scripts/test_cargo_license_state.py`, `scripts/verify_cargo_license_state.py` et Clippy ciblé passent. Clippy sort 22 avertissements de la bibliothèque partagée; aucun ne pointe vers les fichiers touchés. Cargo garde `license=null`, `license_file=null`, `publish=[]`, `no_license_selected`, redistribution `not_assessed`, SPDX `NOASSERTION`.

Un replay du test composé `mission_012_signed_request_context_red_green_independent_verification_and_sandbox_merge` a aussi été tenté avec le runner isolé. Il échoue avant la vérification indépendante : le premier appel MCP `cargo.test` obtient `Failed to connect to bus: No medium found` depuis `systemd-run --user`; le processus retourne donc une baseline inconclusive. Le runner externe masque `/run/user` et son bus. Ce run ne compte pas comme parcours composé réussi; aucun accès au bus hôte n’a été ajouté. Voir `composed-mission-regression.log`.

## Limites et sécurité

Les pins sont compilés pour la seule fixture MISSION-012, pas résolus depuis un registre générique d’exigences. Le vérificateur est un processus distinct mais réutilise le même module `mission_012_host.rs` que le host MCP : l’implémentation n’est pas une deuxième codebase. Le processus hôte appelant fournit toujours les chemins du dépôt local/base/candidat et les deux SHA; le résultat lie les arbres à ce dépôt fourni, mais n’établit pas l’identité d’un dépôt distant, une PR autorisée ou un commit post-fusion. Aucun candidat ne reçoit le binaire hôte ou les credentials du gate selon les tests de confinement déjà cités dans le checkpoint.

La migration GitLab PC4 reste bloquée. Le remote `pc4-gitlab` est déjà configuré, mais le nom MagicDNS ne résout pas depuis ce poste et le TCP HTTPS/443 expire même via l’IP Tailscale; `tailscale ping pc4` réussit. Le courriel fournit un jeton limité à deux projets Nexora sans accès au dépôt cible; ce jeton, déjà signalé comme exposé, n’a pas été réutilisé. Le mot de passe root n’a pas été utilisé. Aucun fetch/authentification/commit/push GitLab n’a eu lieu. Une route HTTPS joignable et un nouveau credential limité à `root/gitspace-forgejo` sont nécessaires avant l’audit de refs et le transfert.

Prochaine action : définir comment le host protège la sélection du dépôt/ref/commit confiés au vérificateur, puis exécuter la composition sans D-Bus hôte exposé au candidat. Continuer en local tant que le dépôt GitLab n’est pas accessible. Projet incomplet; aucun benchmark comparatif ou résultat LLM n’est revendiqué.
