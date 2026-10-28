# CONTINUATION-272 — refuser le credential trop volumineux et relire l’état Cargo

Date: 2026-10-27  
Worktree: `.runtime/webhook-005-workspace`  
HEAD de base: `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`; dépôt très modifié, aucun commit créé.

## Refus du credential trop volumineux

Le scénario `actual_reconcile_cli_process_refuses_oversized_credential_pipe_without_leaking_bytes` démarre le receiver signé courant, ajoute deux événements `issues/opened`, arrête le receiver et passe au vrai CLI un pipe anonyme contenant 4096 octets synthétiques suivis d’un marqueur. Le contenu dépasse la borne de lecture de 4096 octets. Le scénario ciblé passe 1/1 via `scripts/run_webhook_reconcile_cli_e2e.py` dans Bubblewrap 0.9.0 avec `--unshare-net --unshare-pid`, source read-only, HOME et TMP masqués et Cargo offline/verrouillé.

Le CLI sort avec le code 1, `host_credential_pipe_read_failed` et verdict `inconclusive`. `reader_error` est le diagnostic statique « host credential input exceeds the configured byte limit »; l’appel upstream vaut zéro, aucun état canonique ni `results` n’est rendu, les deux sources restent liées par ID/séquence/hash avec attestation observée `verified`, stderr reste vide, le marqueur n’apparaît dans aucune sortie et le journal reste byte-identical. Cette tranche ajoute un test processus et son entrée dans la liste du runner; le runtime n’a pas été modifié.

## État technique des métadonnées de licence

Relecture fraîche de `agent-forge-core 0.1.0`: `cargo metadata --locked --offline --no-deps --format-version 1` et `scripts/verify_cargo_license_state.py` confirment `license=null`, `license_file=null`, `publish=[]`, `decision=no_license_selected`, `redistribution_rights=not_assessed` et le marqueur personnalisé `NOASSERTION`. Les 12 tests de `scripts/test_cargo_license_state.py` passent, dont les refus des champs omis, d’une licence déclarée, d’un fichier de licence, et d’une publication activée. Aucun champ Cargo n’a été ajouté ou modifié; aucun choix, droit ou avis juridique n’est inféré.

## Vérification et limites

La suite `cargo test --locked --offline --all-targets` passe **688 tests, 0 échec, 148 ignorés** sur 37 blocs. `cargo fmt --all -- --check`, `py_compile` des scripts concernés et `git diff --check` passent. Les reçus JSON, logs, commandes, toolchain et empreintes sont dans ce dossier.

Ce test de limite s’exécute localement avec un receiver et un CLI réels; aucune instance Forgejo n’est lancée et aucun service/modèle externe n’est contacté. Il ne prouve pas l’identité du writer, la concurrence hostile, le parcours LLM/AgentOS, un gain comparatif ou des droits de redistribution. Le fichier processus et le runner sont dans l’état Git non suivi déjà fortement modifié; aucun commit n’a été créé.

Suite: décider et tester explicitement le contrat d’un FIFO nommé, puis choisir une lacune de comportement du parcours adapter/réconciliation encore ouverte dans le checkpoint. La mission reste incomplète.
