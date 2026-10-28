# CONTINUATION-206 — le profil de réconciliation-only ne passe pas à l’autorité de fusion

Run de contrat/processus sur le worktree `webhook-005-workspace`, base HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`, préalablement fortement modifié.

Le parseur d’options de `--merge-local-test` n’accepte que `--target-profile` comme sélecteur de cible. Le nouveau test unitaire appelle la vraie entrée `run_local_test_merge_authority` avec `--reconcile-target-profile org-team-review-e2e` et un pipe contenant un token synthétique. La fonction renvoie `unknown --merge-local-test option`; le test lit ensuite le pipe et compare les octets du token, qui n’ont pas été consommés.

Un second cas soumet la valeur `org-team-review-e2e` au sélecteur merge autorisé `--target-profile`; le parseur renvoie `unknown Forgejo target profile` et le pipe-token reste intact. Les deux cas sont aussi exercés en processus réel via `CARGO_BIN_EXE_forgejo-reconcile-local`: le premier emploie l’option `--reconcile-target-profile` inconnue du mode merge, le second le bon sélecteur merge avec la valeur réservée à la réconciliation. Ils vérifient chacun un code non zéro, stdout vide et le diagnostic exact, avec stdin vide et sans fichier de preuve ni credential. Les profils fusion restent issus de `LocalForgejoTargetProfile::ALL`; le profil d’organisation n’est pas ajouté à cette énumération.

Résultats : les deux tests unitaires ciblés passent 2/2 et les deux tests processus 2/2; `cargo fmt --all -- --check` et `git diff --check` passent. Les logs et commandes sont dans ce dossier. Ce lot ajoute les refus de contrat et de processus; aucune règle d’autorisation n’est modifiée. Il ne lance pas Forgejo, ne teste pas le réseau ni un merge réel. Il n’établit pas que toutes les voies d’entrée ou tous les writers sont isolés; il prouve précisément le refus de ce sélecteur dans ce parseur avant consommation du pipe synthétique.

Aucun LLM/AgentOS, service externe, coût, publication, production ou code upstream. Le projet reste incomplet; les parcours de fusion, post-merge et comparaison appariée restent ouverts.
