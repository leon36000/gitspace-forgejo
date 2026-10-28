# CONTINUATION-296 — provenance de l’arbitrage humain des exigences

Date : 27 octobre 2026. Audit en lecture seule du worktree local fondé sur `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`, avec un arbre fortement modifié. Aucun code produit, service, processus préexistant, modèle, credential, accès production ou état externe n’a été utilisé. Aucun test n’a été exécuté.

## Constat

`src/lib.rs::Requirements` (schéma v2) contient `source_issue_number`, `request`, `acceptance` et `ambiguities`, sans décision résolue, justification ou identité d’arbitre. `src/mission_admission.rs::MissionAdmissionSourceV1` lit issue, état de protection, octets du fichier au commit et inventaire des chemins; son interface ne retourne pas de PR source, avis, identité de réviseur ni preuve d’autorisation. Dans `prepare_admission`, les exigences sont liées à l’issue, au commit et au SHA; une liste `ambiguities` non vide est refusée avant allocation. Lorsque la liste est vide, le code poursuit vers l’allocation sans contrôler l’origine humaine de cette révision.

Le test `tests/mission_admission_contract.rs::unresolved_human_ambiguity_refuses_admission_before_generation_allocation` couvre l’ambiguïté encore ouverte, pas une résolution attribuée à un réviseur Forgejo. `src/forgejo.rs::validate_merge_approvals` et `MergeApprovalPolicyV1` offrent une allowlist d’identifiants et une vérification d’avis pour la PR candidate. Le commentaire de cette fonction précise que cette évaluation n’authentifie pas le lecteur Forgejo, ne prouve pas que les comptes sont humains et n’est pas une autorisation de fusion. Elle n’établit pas non plus le lien entre une exigence protégée et la PR qui l’a modifiée.

Ce constat confirme la limite déjà signalée autour de CONT-242; ce n’est pas un test d’un acteur Forgejo non autorisé et ne prouve pas qu’un accès réel de ce type existe. Un champ de nom d’auteur ou une signature incluse uniquement dans le JSON d’exigences ne fermerait pas le défaut, puisqu’il n’authentifierait pas l’autorité de la source.

## Réconciliation du constat d’egress

Le prochain travail automatisé proposait de refaire une vérification de l’accès direct de l’agent à Forgejo. Les rapports [CONTINUATION-285](../continuation-285-current-stdio-mcp-webhook-boundary-001/local-run.md) et [CONTINUATION-286](../continuation-286-webhook-attestation-seed-sandbox-probe-001/local-run.md) couvrent déjà un processus stdio-MCP réel de fixture dans Bubblewrap pendant un gate Forgejo local; le gate fusionne et relit la PR tandis que l’agent ne joint pas les routes sondées et ne lit pas le credential de fusion. CONT-286 ajoute l’ouverture refusée de la graine Ed25519 effectivement utilisée par le receiver.

Les SHA256SUMS locaux de CONT-285/286 sont valides. La revalidation de `source-sha256.txt` de CONT-286 confirme que ses fichiers runtime clés (`mission_012_sandboxed_test_adapter.py`, `mission_012_test_agent_adapter.py`, `run_forgejo_live_pr_merge_e2e.py`, `run_mission_012_stdio_mcp_egress_gate_e2e.py` et le test sandbox) correspondent toujours aux empreintes testées. Deux documents de CONT-286 ont depuis changé, comme attendu; leurs anciens hashes ne sont pas présentés comme courants. Le résultat egress reste `n=1`, local-test-adapter, trois routes ciblées, Linux/aarch64 et Bubblewrap 0.9.0. Il ne couvre pas toutes routes, un LLM/AgentOS, les acteurs privilégiés ou un noyau compromis.

## Suite proposée et limites

Vérifier dans le code Forgejo v17 et les interfaces courantes de l’adaptateur si un commit contenant les octets exacts d’un fichier d’exigences peut être lié à une PR fusionnée et à son snapshot de revue courant, puis si une politique hôte protégée peut autoriser un ensemble d’identifiants de réviseurs. Réutiliser `MergeApprovalPolicyV1` seulement si la provenance et la fraîcheur de cette liaison sont établies. La frontière `MissionAdmissionSourceV1` devra refuser avant allocation quand la preuve manque ou ne correspond pas à l’issue, au fichier, au commit, au hash, à la revue ou à la politique. Ce rapport propose un contrôle source/API, pas une nouvelle API adoptée ni une validation d’identité humaine réelle.

Le lot a actualisé l’inventaire, la matrice des refus, l’architecture et le checkpoint afin de supprimer une assertion d’egress périmée et de distinguer l’état des exigences de l’autorisation d’arbitrage. Vérification comportementale et intégration d’une preuve Forgejo d’arbitrage restent à faire.
