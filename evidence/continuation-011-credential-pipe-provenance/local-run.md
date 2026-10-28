# CONTINUATION-011 — provenance du FD credential

Date locale : 25 octobre 2026. Workspace : `.runtime/webhook-005-workspace`, HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. L’état Git est largement modifié avant ce lot; seules les surfaces citées ici ont été éditées. Les versions sont dans `tool-versions.txt`.

## Lacune constatée

`read_private_credential_pipe` vérifiait le numéro de FD, le type FIFO et la cible `/proc/self/fd/N` `pipe:[…]`, imposait une lecture bornée et validait le token. Ces opérations attestent une forme de pipe et un format de token. Elles n’authentifient pas le processus ayant fourni le FD et ne prouvent pas qu’un seul processus détient une extrémité lisible. Un test existant duplique le lecteur et confirme que la lecture du pipe peut être affectée par ce lecteur concurrent. Le terme « private » dans le contrat de capacités v2 et dans les fonctions internes était donc trop fort.

## Changement

- Remplacement des valeurs publiques `private_inherited_pipe_fd` par `inherited_anonymous_pipe_fd`.
- Ajout du contrat partagé `credential_boundary.credential_fd_validation`: pipe anonyme vérifié, contenu lu jusqu’à EOF sous une limite de 4096 octets et de 2000 ms, syntaxe du token validée, identité producteur non authentifiée, propriété exclusive non vérifiée.
- Les opérations réconciliation et fusion pointent explicitement vers ce contrat commun.
- Renommage des helpers internes et correction des erreurs pour ne plus suggérer de confidentialité du FD.
- Aucun changement d’autorité, d’isolation, de permissions Forgejo ou de comportement de fusion.

## Vérifications exécutées

- `cargo test --locked --offline --bin forgejo-reconcile-local capabilities_list_exact_merge_target_profiles`: 1 réussi.
- `cargo test --locked --offline --bin forgejo-reconcile-local host_credential_pipe`: 2 réussis; inclut le test du lecteur dupliqué et la lecture qui expire si le writer garde le pipe ouvert.
- `cargo test --locked --offline --bin forgejo-reconcile-local host_credential_reader_rejects_named_fifo`: 1 réussi.
- `cargo test --locked --offline --test forgejo_reconcile_capabilities_e2e actual_cli_discloses_versioned_local_test_capabilities`: 1 réussi sur le vrai processus CLI.
- `cargo build --locked --offline --bin forgejo-reconcile-local`: réussi. La sortie `--capabilities` capturée dans `capabilities.json` a été relue par Python; les limites, les bornes et références des deux opérations correspondent.
- `cargo fmt --all -- --check` et `git diff --check`: réussis après correction d’une ligne de formatage.

Les journaux de commandes, la sortie JSON et les empreintes sont dans ce dossier. Une première vérification locale de JSON a ciblé à tort la référence de réconciliation au niveau racine au lieu de `credential_contract`; son traceback est conservé sous `capabilities-readback-check-error.log`. La vérification relancée a inspecté le chemin réel et passé. Une première commande de test avec `-- --exact` a filtré zéro test; la commande corrigée ci-dessus a exécuté 1/1. La première vérification rustfmt a trouvé une ligne longue, corrigée avant la vérification finale.

## Couverture et limites

Ces checks valident l’implémentation de la lecture et l’honnêteté de la découverte locale. Ils n’authentifient pas le parent, ne créent pas de sandbox, ne séparent pas l’agent du CLI, ne prouvent pas l’absence d’un FD dupliqué et ne testent pas un secret réel. Un processus agent ayant accès au processus hôte ou au FD peut toujours lire le credential. L’appelant doit fournir le FD uniquement au processus hôte de confiance et maintenir cette séparation au niveau OS.

Aucun Forgejo live, accès extérieur, credential réel, LLM, AgentOS, service, coût ou comparaison conventionnelle n’a été utilisé. La tâche globale reste incomplète. Suite utile : reprendre dans le scratch isolé l’audit des writers Forgejo et le protocole de publication LFS, sans adopter le patch expérimental.

## Intégrité des artefacts

Le manifeste du dossier passe pour ses 15 artefacts. Après ajout append-only, les dernières empreintes du manifeste du worktree ont été relues pour 21 chemins et celles de l’index parent pour 22 chemins (incluant le manifeste du worktree); les anciennes lignes malformées sont préservées, 7 dans l’index local et 2 dans l’index parent.
