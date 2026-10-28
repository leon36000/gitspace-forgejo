# CONTINUATION-150 — autorité de fusion et séparation des credentials au code courant

Date : 26 octobre 2026. HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`; le worktree est modifié et préservé. Les hashes des fichiers examinés sont dans `source-sha256.txt`.

## Constat de code

- L’autorité qui parle à Forgejo est la commande `forgejo-reconcile-local --merge-local-test` dans `src/bin/forgejo-reconcile-local.rs`. `src/bin/gate.rs` signe/vérifie des preuves locales et ne constitue pas le gate réseau de fusion.
- Le profil par défaut `forgejo_v17_multiwriter` est refusé avant verrou, lecture d’inputs, lecture de credential ou POST parce que Forgejo v17 ne fournit pas ici de précondition serveur obligatoire. `local_single_writer_sandbox` exige explicitement une capacité superviseur à usage unique, liée par peer credentials et identités de processus/namespaces. La CLI reçoit/valide cette capacité avant d’appeler `read_inherited_credential_pipe`; elle la revalide avant le POST.
- Les métadonnées de capacité disent elles-mêmes que l’exclusion de tous les writers et la politique du superviseur ne sont pas indépendamment vérifiées par l’adaptateur. Cette limitation reste un état de confiance hôte, pas une garantie Forgejo.
- `mission_012_mcp_supervisor.py` démarre son hôte MCP avec un environnement dédié et `close_fds=True`, sans `pass_fds`. L’adaptateur d’agent testé par le canari reçoit `--clearenv`, home/tmp privés, aucune montée de credential et `close_fds=True`. Le contrôle live CONTINUATION-149 confirme les trois refus de route et l’absence du marqueur synthétique dans le processus isolé.

## Vérifications rattachées

Le binaire `forgejo-reconcile-local` passe 151/151 et l’ensemble Rust du worktree 630 réussis, 0 échec, 111 ignorés sur 36 blocs (CONTINUATION-148). Dans ces tests figurent `merge_cli_refuses_invalid_supervisor_descriptor_before_reading_credential`, qui laisse le pipe contenant une valeur synthétique intact après refus, `merge_cli_defaults_to_refusing_multiwriter_before_reading_merge_credential`, `merge_cli_refuses_single_writer_profile_without_supervisor_capability_before_credential` et `merge_gate_uses_separate_policy_reader_and_non_admin_merge_credentials`. Le canari de processus au snapshot courant et son reçu sont documentés dans CONTINUATION-149.

## Portée et limites

C’est une revue de source liée à des tests Rust et au canari d’egress, sans nouveau merge ni nouveau parcours LLM. Le canari positif est un GET avec un credential lecture seule dans le processus hôte, pas un POST de gate. Les tests Rust de credentials emploient des fixtures/synthétiques. Le parcours fusionné 009 reste historique et ses sources ont dérivé; on ne l’emploie pas comme validation du merge courant. L’isolation ne couvre pas un même UID hostile, root, acteurs privilégiés, noyau compromis, évasion, ni une politique prouvée contre tous les writers. Aucun credential réel n’a été lu ni imprimé, aucune production, publication ou donnée externe n’a été touchée.

Suite : maintenir le refus par défaut Forgejo v17 multiwriter; pour avancer vers une fusion actuelle, concevoir une vérification live isolée du gate avec Forgejo local après inspection du runner et des contrats actuels, sans réutiliser l’ancienne affirmation de contrôle exclusif. La mission reste incomplète.
