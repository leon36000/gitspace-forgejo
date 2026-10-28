# CONTINUATION-273 — refuser un FIFO nommé comme source du credential

Date: 2026-10-27  
Worktree: `.runtime/webhook-005-workspace`  
HEAD de base: `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`; dépôt fortement modifié, aucun commit créé.

## Résultat observé

Le scénario `actual_reconcile_cli_process_refuses_named_fifo_as_host_credential_source` crée un FIFO nommé mode 0600, l’ouvre en lecture non bloquante sans writer, puis passe explicitement son FD au vrai reconciler. Le receiver local signe et journalise deux événements avant cette invocation. Le test s’exécute via le runner existant sous Bubblewrap 0.9.0, `--unshare-net --unshare-pid`, source read-only et Cargo offline/verrouillé.

Le processus passe 1/1 en 0,07 s. Il refuse avec code 1, JSON `inconclusive`/`host_credential_pipe_read_failed`, diagnostic statique `host credential input must be an anonymous pipe`, lecture upstream nulle, aucune projection canonique ou ligne `results`. Les deux sources sont liées par ID/séquence/hash et l’observation d’attestation est `verified`; le journal est inchangé et stderr vide. Le CLI distingue donc le FIFO nommé de l’anonymous pipe exigé, sans attendre un writer.

Le premier run conservé dans `process.log` a échoué parce que `OpenOptions` avait gardé `FD_CLOEXEC` sur le FD du FIFO; le CLI recevait un descripteur fermé et rapportait `cannot secure the host credential descriptor`. Le fixture de test a ensuite explicitement effacé `FD_CLOEXEC`, comme pour transmettre un descripteur hérité, et le run final vert est conservé dans `process-final.log`. Ce diagnostic initial concernait le test setup, pas une régression du reconciler.

Suite finale `cargo test --locked --offline --all-targets`: **688 réussis, 0 échec, 149 ignorés** sur 37 blocs (`all-targets.log`). `cargo fmt --all -- --check`, py_compile du runner et `git diff --check` passent. Les commandes et la toolchain sont archivées ici.

## Limites

Test processus local Linux avec receiver synthétique, sans Forgejo live, LLM ou AgentOS. Aucun code runtime n’a été modifié. Cela prouve le refus de ce FIFO précis, pas un contrat de plateforme portable, une identité de producteur, une concurrence hostile ni une protection contre un hôte privilégié. Le contrôle d’inodes `pipe:[...]` est une observation locale, pas une authentification du producteur. Aucun benchmark n’est ajouté.

Suite: les quatre cas credential (input mal formé, timeout, dépassement de taille, FIFO nommé) ont maintenant des tests processus séparés; choisir une lacune fonctionnelle du parcours adapter/réconciliation ouverte au checkpoint plutôt que répéter ces tests.
