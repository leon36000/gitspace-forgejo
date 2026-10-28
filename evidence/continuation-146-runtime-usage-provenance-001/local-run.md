# CONTINUATION-146 — provenance des tokens et coûts rapportés

Date : 26 octobre 2026. Audit statique et test de contrat dans le worktree local, HEAD examiné `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`, branche `local/WEBHOOK-005-canonical-links`, checkout fortement modifié. Aucun commit, reset, service Forgejo, modèle, accès externe, coût ou credential.

## Résultat

`src/agent_runtime.rs::AgentUsageV1` transporte trois options (`input_tokens`, `output_tokens`, `cost_microusd`). `None` signifie indisponible, pas zéro. `AgentExecutionResultV1::validate_for` compare seulement la valeur `output_tokens` rapportée à `max_output_tokens` : il peut refuser une déclaration au-dessus de la limite, mais ne peut établir qu'une déclaration faible ou nulle reflète l’usage réel. Le schéma n’a pas de limite de coût ni de reçu fournisseur lié à la réponse. `RuntimeBudgetV1::max_output_tokens` exprime donc la demande au runtime; le résultat seul ne prouve pas l’application du plafond.

Recherche exécutée : `rg -n 'impl AgentRuntimeV1' src tests scripts` ne trouve que `tests/agent_runtime_contract.rs::FixtureRuntime`. Le test ajouté `runtime_claimed_zero_usage_is_not_a_host_usage_receipt` exécute trois appels d’outils de la fixture, remplace les trois métriques par `Some(0)`, puis vérifie que `validate_for` accepte toujours le résultat. Cela prouve la limite de provenance du contrat; cela ne simule pas ni ne mesure un modèle.

Les commentaires d’interface, le plan de mesure et la matrice de refus précisent maintenant que ces champs ne sont pas une preuve de coût, de tokens ou de budget dur. Un futur adaptateur fournisseur devra appliquer son plafond pendant la génération et produire une observation d’usage indépendante liée à la mission et à la génération. Tant qu’il est absent, ces résultats ne conviennent pas à un benchmark quantitatif et l’absence d’une mesure ne vaut pas zéro.

## Vérifications

- `cargo test --offline --locked --test agent_runtime_contract` : 11 réussis, 0 échec, 0 ignoré.
- `cargo fmt --all -- --check` : code de sortie 0.
- `git diff --check` : code de sortie 0.
- Un contrôle Python des espaces finaux sur `src/agent_runtime.rs`, `tests/agent_runtime_contract.rs`, `docs/architecture.md`, `docs/checkpoint.md` et `docs/refusal-evidence-matrix.md` : aucun espace final.
- `rustc 1.95.0`, `cargo 1.95.0`, Linux `7.0.0-1019-nvidia aarch64`.

Les commandes et sorties sont conservées dans ce dossier; `source-sha256.txt` épingle les sources/documentations après modification et `SHA256SUMS` couvre les artefacts du rapport. Les index worktree/projet ont été étendus après capture et vérification de leurs préfixes précédents. Les manifests sont append-only et conservent d’anciennes empreintes pour les chemins dont le contenu évolue; une vérification qui contrôle toutes les lignes avec `sha256sum -c` signalera donc ces anciennes versions. Le vérificateur du lot contrôle la dernière empreinte par chemin, chaque fichier local et les préfixes historiques; il passe pour 13 fichiers de preuve et confirme que l’index parent lie l’index worktree courant.

## Portée et limites

Un test de contrat sous fixture, sans consommateur runtime hors de `tests`, ne vérifie ni l’usage réel d’un fournisseur, ni l’authenticité de ses reçus, ni l’arrêt à l’atteinte d’un plafond, ni la facturation. Aucune exécution LLM/AgentOS, PR, fusion, vérification indépendante du code candidat ou mesure comparative n’a eu lieu. Le résultat ne modifie aucun comportement de fusion ou de vérification. La mission reste incomplète.
