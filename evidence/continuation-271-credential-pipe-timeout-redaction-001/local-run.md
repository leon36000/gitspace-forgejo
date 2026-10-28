# CONTINUATION-271 — timeout borné d’un pipe credential laissé ouvert

Date: 2026-10-27  
Worktree: `.runtime/webhook-005-workspace`  
HEAD de base: `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`; worktree fortement modifié, aucun commit créé.

## Question vérifiée

Le lecteur du FD credential fixe `HOST_CREDENTIAL_PIPE_READ_TIMEOUT` à deux secondes. Le lot teste le vrai chemin processus lorsque le pipe existe, reste vide et n’atteint jamais EOF avant expiration. Il n’altère pas le code runtime.

## Résultat

Le scénario `actual_reconcile_cli_process_times_out_open_credential_pipe_and_binds_sources` démarre le receiver signé, ingère deux événements `issues/opened` valides, ferme le receiver puis invoque le vrai binaire avec un reader de pipe dont le writer reste ouvert sans contenu. Le processus est exécuté via `scripts/run_webhook_reconcile_cli_e2e.py` sous Bubblewrap 0.9.0 (`--unshare-net --unshare-pid`, workspace en lecture seule, `/home` et `/tmp` tmpfs, toolchain et Cargo offline/verrouillés).

Le test passe 1/1; le délai observé est de 2,07 secondes. Le CLI sort avec le code 1 et rend un JSON batch v17 `inconclusive`, `host_credential_pipe_read_failed`, `reader_error="timed out waiting for host credential pipe EOF"`, lecture du credential tentée mais échouée, zéro appel upstream, aucune projection canonique ni ligne `results`. Les deux entrées `selected_event_sources` restent ordonnées et liées aux séquences, hashes et attestations vérifiées. Après la fermeture du writer, le reader parent atteint EOF sans octet; le journal est identique au snapshot avant invocation; stderr est vide.

La suite finale `cargo test --locked --offline --all-targets` passe **688 tests, 0 échec, 147 ignorés** sur 37 blocs (`all-targets.log`). Le processus ciblé passe 1/1 en 2,07 s (`process.log`). `cargo fmt --all -- --check`, `py_compile` du runner et `git diff --check` réussissent; sorties conservées. Commandes et versions sont dans `commands.txt` et `toolchain.txt`.

## Limites

Le test établit seulement le timeout de lecture du pipe local après que les lignes sources sont sélectionnées. Il n’éprouve pas une panne machine, la taille excessive, un FIFO nommé, un writer concurrent hostile, un processus privilégié, l’identité du writer local ou une instance Forgejo. Aucun credential réel, Forgejo live, LLM ou AgentOS n’est utilisé. Aucun benchmark ni amélioration générale n’est démontré.

Suite: tester la borne de taille maximale avec un input synthétique, puis vérifier séparément la décision de contrat pour les FIFO nommés. Poursuivre l’examen des frontières d’autorité sans dépense modèle.
