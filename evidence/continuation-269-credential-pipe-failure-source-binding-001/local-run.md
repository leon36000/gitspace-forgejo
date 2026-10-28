# CONTINUATION-269 — refus structuré d’échec du FD credential

Date : 27 octobre 2026. Worktree local au HEAD de base f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0, fortement modifié. Aucun commit, reset, nettoyage, publication, changement de production, secret fournisseur ou accès élargi.

## Changement

Quand les IDs sont entièrement sélectionnés et les attestations requises vérifiées, l’échec du lecteur du credential ne remonte plus comme erreur CLI non structurée. Le reconciler retourne inconclusive avec le code host_credential_pipe_read_failed, upstream_read_attempts=0, aucun results ni état canonique, et un diagnostic reader_error. Les octets du credential ne sont pas ajoutés au JSON. Les refus batch conservent selected_event_sources avec ID, séquence, hash et observation d’attestation; single garde journal_source. Le schéma batch passe à v17, les capacités à v15, single reste v13. La découverte des capacités annonce le refus et la liaison de source.

Le diagnostic fourni par read_inherited_credential_pipe est actuellement une chaîne constante ne contenant pas les octets lus. L’API expose ce diagnostic comme champ actionnable; cette propriété dépend de la discipline future de garder les messages du lecteur non dérivés du credential.

## Preuves

TDD : rust-red.log échoue parce que run_reconciliation retourne Err("cannot secure the host credential descriptor") au lieu d’un JSON; process-red.log montre le vrai CLI laisser une erreur sur stderr, faisant échouer l’assertion de sortie structurée. Après implémentation, rust-green.log passe 1/1.

Le scénario de processus réel actual_reconcile_cli_process_reports_credential_pipe_read_failure_with_selected_sources passe 1/1 sous Bubblewrap (process-final.log). Le receiver Rust réel accepte deux webhooks signés, puis le vrai CLI vérifie la clé publique avant d’essayer le descripteur volontairement invalide i32::MAX. Il écrit le refus JSON sur stdout, rien sur stderr; les deux IDs sont liés aux séquences/hashes et portent verified pour la seule signature receiver; aucune signature, aucun résultat individuel, aucun état canonique, aucun read upstream ni mutation du journal. Le processus est dans --unshare-net --unshare-pid; l’état upstream zéro est aussi asserté.

Vérification finale : cargo test --locked --offline --all-targets = 688 réussis, 0 échec, 145 ignorés sur 37 blocs; binaire forgejo-reconcile-local = 161/161; contrats ciblés capacités/configuration/quarantaine = 5 + 4 + 5 réussis, 2 ignorés; 3 consommateurs Python = 190/190; cargo check --all-targets, rustfmt, py_compile, git diff --check, vérification Cargo et 12 tests de garde licence passent. Le premier run complet ciblé des tests avait révélé que capabilities_document n’exposait pas encore le champ pipe_read_failure; le code et l’assertion ont été alignés puis les suites vertes rejouées. Le diagnostic est conservé dans capabilities-contract-diagnostic.md.

Toolchain : Linux aarch64, rustc/cargo 1.95.0, Python 3.12.3, Bubblewrap 0.9.0. Les commandes détaillées, le résultat Cargo, les reçus et les hashes sont dans ce dossier.

## Portée et limites

Le processus teste spécifiquement un FD invalide : échec de validation par fcntl avant toute lecture d’octet. Il n’exerce pas en processus le timeout, la taille excessive, le FIFO nommé ou le rejet de syntaxe d’un pipe correctement ouvert; ces erreurs passent par la même branche Result, mais cette généralisation n’est pas une preuve d’intégration. L’enveloppe single est implémentée mais ce lot valide les assertions nouvelles sur batch. Les refus antérieurs à la sélection complète n’ont pas de liste de sources.

Cette observation atteste la signature du receiver local sous sa clé de confiance, pas l’identité du writer de journal, la véracité amont ou la correction du logiciel. Aucun Forgejo live, LLM, AgentOS, RAG, mémoire ou benchmark n’a été utilisé. Cargo reste license=null, license_file=null, publish=[], no_license_selected, droits de redistribution not_assessed, export personnalisé NOASSERTION; aucune licence ou permission de redistribution n’est choisie.

Prochaine étape : tester les autres classes d’erreur du pipe avec une fixture synthétique sans révéler les octets, puis obtenir une preuve indépendante que les writers API/Web/receive-pack et le credential de fusion restent hors de portée de l’agent. La comparaison Forgejo + agent + CI conventionnels reste sans runtime/budget autorisé et ne démontre aucun gain.
