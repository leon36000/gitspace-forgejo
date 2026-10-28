# CONTINUATION-050 — annonce machine lisible du fence entre PR concurrentes

Date : 26 octobre 2026. Worktree `/home/pc6/Documents/Codex/2026-10-21-mission-construire-une-forge-con-ue/.runtime/webhook-005-workspace`, HEAD de base `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`, branche `local/WEBHOOK-005-canonical-links`, dépôt toujours fortement modifié.

## Lacune et changement

Le scénario de processus CONTINUATION-049 établit déjà que le vrai CLI retourne `inconclusive`, `retryable=true`, `execution_merge_target_unresolved` pour une intention de fusion non résolue visant une autre PR de la même branche de base, avant tout appel Forgejo. Le descripteur `--capabilities` n’expliquait pas cette erreur : un client devait deviner sa signification et comment la résoudre.

L’opération `local_test_merge_authority` expose maintenant `merge_attempt_scope_fence`, avec la portée `normalized_origin_owner_repository_base_branch`, le code, verdict, caractère réessayable, champs de détail, les refus avant préflight, avant append et lors de la récupération d’une PR, ainsi que leurs actions correspondantes. Le champ avertit explicitement que le fence tient au journal coopératif et n’exclut pas les writers externes Forgejo. Le contrat est ajouté à la découverte v2 comme donnée additive.

## Preuve rouge/verte

Avant le changement, `actual_cli_discloses_versioned_local_test_capabilities` échouait en lançant le vrai binaire : la valeur observée était `null` au lieu du contrat structuré attendu; exit 101. Le résumé fidèle du résultat capturé est dans `red-phase.txt` (ce n’est pas une sortie brute conservée).

Après le changement :

- `cargo test --offline --locked --test forgejo_reconcile_capabilities_e2e actual_cli_discloses_versioned_local_test_capabilities -- --exact` — 1/1 réussi; le processus réel `--capabilities` publie la description complète.
- `cargo test --offline --locked --bin forgejo-reconcile-local capabilities_list_exact_merge_target_profiles` — 1/1 réussi.
- `cargo fmt --all -- --check` et `git diff --check` — réussis.
- `cargo metadata --offline --locked --no-deps --format-version 1` — réussite; `agent-forge-core` reste sans champ `license` ni `license_file`, avec `publish=[]`.

Les sorties finales conservées sont dans ce dossier. Le comportement de refus lui-même est référencé dans [CONTINUATION-049](../continuation-049-cli-branch-scope-fence/local-run.md); ce parcours n’a pas été rejoué.

## Réexamen de l’isolement agent/gate

L’action de la reprise précédente demandait de vérifier l’isolement réseau du processus et le chemin d’autorité. Cette preuve existe déjà en [FORGEJO-AGENT-EGRESS-MERGE-001](../forgejo-agent-egress-merge-001/local-run.md) : une instance jetable Forgejo v17 accepte les requêtes du gate de fusion de confiance pendant qu’un processus Bubblewrap reste vivant, sans FD de credential, observe seulement `[0,1,2]`, et échoue sur les routes API, Web et `git-receive-pack` avant et après la fusion. Ce lot ne l’a pas répété.

La limite conservée par le rapport précédent est réelle : le processus tenu vivant est une sonde déterministe, pas le processus de l’adaptateur qui effectue la tâche ni un LLM/code hostile; l’exclusion multi-tenant et tout trafic réseau ne sont pas prouvés. La prochaine tranche utile serait de composer la vraie exécution du `local-test-adapter` derrière cette frontière, uniquement si cela apporte une observation nouvelle.

## Portée et limites

Cette tranche ne change ni le contrôle du fence, ni l’adaptateur de fusion, ni Forgejo. Elle rend seulement le contrat public plus découvrable et relie la vérification du document aux tests qui prouvent déjà son comportement. Les vérifications utilisent le binaire et des tests locaux; aucune instance Forgejo, Bubblewrap, modèle, AgentOS, service externe ou donnée privée n’a été utilisé ici. Elle ne prouve aucun gain général, isolation universelle ou exclusion des writers externes. Projet toujours incomplet.
