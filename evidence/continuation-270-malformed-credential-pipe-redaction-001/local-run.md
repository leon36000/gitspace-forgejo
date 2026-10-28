# CONTINUATION-270 — refus/redaction d’un credential mal formé dans un pipe ouvert

Date: 2026-10-27  
Worktree: `.runtime/webhook-005-workspace`  
HEAD observé avant le lot: `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0` (worktree fortement modifié; aucun commit créé)

## Question vérifiée

CONTINUATION-269 exerçait l’erreur du descripteur invalide (`fcntl`) mais pas un pipe anonyme lisible dont le contenu échoue à l’analyse. Ce lot caractérise ce chemin avec le vrai processus reconciler et un vrai receiver signé. Il n’ajoute aucun comportement au runtime.

## Exécution et résultat

La commande reproductible est dans `commands.txt`. `scripts/run_webhook_reconcile_cli_e2e.py` compile et lance le test processus via Bubblewrap 0.9.0, avec `--unshare-net --unshare-pid`, workspace source en lecture seule, `/home` et `/tmp` tmpfs, toolchain Rust monté en lecture seule et Cargo offline/verrouillé. Le processus test démarre `forgejo-webhook-local`, poste deux événements `issues/opened` HMAC valides au receiver configuré avec attestation Ed25519, arrête le receiver, relit les lignes, puis crée un pipe anonyme. Il écrit `synthetic-malformed-credential MUST_NOT_LEAK_continuation270`, ferme le writer et passe le reader au binaire `forgejo-reconcile-local` avec la clé publique de confiance et les deux IDs.

Le test `actual_reconcile_cli_process_redacts_malformed_open_credential_pipe_and_binds_sources` passe 1/1. La sortie du reconciler a le code de sortie 1, JSON structuré `host_credential_pipe_read_failed`, verdict `inconclusive`, `credential_pipe_read_attempted=true`, `credential_pipe_read_succeeded=false`, `upstream_calls_attempted=false`, `canonical_state_created=false` et zéro lecture upstream. `selected_event_sources` conserve l’ordre ID → séquence → hash → statut d’attestation `verified`; aucune signature ou ligne `results` n’est incluse. Le champ `reader_error` contient le diagnostic statique du parseur. Le marqueur synthétique n’apparaît ni dans stdout ni dans stderr; le reader du parent atteint EOF après lecture par le child et les octets du journal restent identiques à leur snapshot.

La commande finale processus est conservée dans `process-final.log`. La première exécution est dans `process.log`. Après rustfmt, la suite `cargo test --locked --offline --all-targets` passe **688 tests, 0 échec, 146 ignorés**, sur 37 blocs (`all-targets-final.log`). Le nouveau processus est ignoré par la suite Cargo ordinaire et doit être lancé via le runner Bubblewrap. `cargo fmt --all -- --check`, `py_compile` du runner et `git diff --check` passent; sorties: `rustfmt.log`, `pycompile.log`, `diff-check.log`. `all-targets.log` conserve aussi la première suite, exécutée avant la seule normalisation rustfmt du nouveau test.

## Environnement

Versions complètes: `toolchain.txt` — rustc et cargo 1.95.0 (aarch64), Bubblewrap 0.9.0, Linux aarch64. Le test est local et hors ligne; aucun service Forgejo n’a été lancé ou contacté, aucun fournisseur LLM, AgentOS, token réel ou credential de fusion n’a été utilisé. Le credential de la fixture est synthétique et mal formé.

## Limites et suite

Ce test couvre une erreur de parsing dans un pipe ouvert après sélection de sources, pas timeout, entrée surdimensionnée, FIFO nommé, writer hostile concurrent, ni identité du writer local. La séparation Bubblewrap ne prouve rien contre un hôte privilégié, un noyau compromis ou un processus du même UID hors sandbox. Il ne démontre pas un parcours LLM, AgentOS, correction de logiciel ou amélioration comparative. Les octets sont consommés avant refus; seul le texte diagnostique statique est rendu.

Suite locale proposée: exercer séparément les refus timeout et taille excessive et comparer leurs diagnostics/redaction. La comparaison Forgejo + agent + CI reste non établie; aucun coût externe n’a été engagé.
