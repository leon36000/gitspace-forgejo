# CONTINUATION-330 — transmettre les labels d’issue actuels au contexte MCP non fiable

Date : 28 octobre 2026. Worktree : `/home/pc6/Documents/Codex/2026-10-21-mission-construire-une-forge-con-ue/.runtime/webhook-005-workspace`. Révision de base du worktree : `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`, branche `local/WEBHOOK-005-canonical-links`; le worktree contient de nombreux changements antérieurs suivis et non suivis. Aucun commit, reset, checkout, push, accès à distance ni changement de production n’a été fait.

## Résultat

Le CLI d’admission de mission transfère maintenant au host MCP les labels provenant de la lecture API courante de l’issue, dans `untrusted_work_item` comme contexte optionnel et non fiable. Il n’utilise jamais comme source canonique les labels possiblement périmés du webhook. La projection ne conserve que `id` et `name`, trie par identifiant, borne la liste à 256 et le nom à 255 octets, exige des ID positifs uniques et des noms non vides sans caractères de contrôle. `labels` absent ou `null` reste inconnu dans l’enveloppe v1; un tableau vide produit une enveloppe v2 connue vide. Le digest v2 inclut les labels et détecte leur altération; il n’atteste pas provenance, vérité ni correction.

L’enveloppe de work item v2 est liée à la requête d’exécution schema 3. Les requêtes 1/2 restent compatibles avec l’enveloppe legacy v1, et les exigences schema 3 sans labels connus peuvent encore utiliser le chemin de requête compatible. L’admission compte l’objet optionnel dans le budget de contexte et peut l’omettre sans modifier les exigences protégées. Le host MCP remet titre, corps et labels dans un champ séparé et annonce qu’ils sont contrôlés par l’utilisateur et ne modifient ni exigences, ni permissions, ni politique de vérification, ni autorité de fusion.

Deux coutures adjacentes sont testées, sans les présenter comme un seul test-processus composé : (1) le vrai receiver/CLI Rust, sous runner Bubblewrap et fixture HTTP loopback, reçoit un webhook synthétique avec label périmé ID 99, lit l’API fixture où le label courant est ID 7 et conserve ID 7 dans le résultat d’admission; le même scénario vérifie issue fermée, retry identique sans double allocation et conflit d’édition après claim sans mutation du journal/handoff; (2) le test de cycle du vrai host MCP Rust reçoit un `MissionHostAdmissionV1` avec labels, sert `mission_context` et vérifie le champ séparé, le trust boundary et les exigences inchangées. L’adaptateur Python déterministe est testé contre enveloppes v1/v2 fabriquées par son faux client MCP; il observe le contexte hostile sans utiliser son nom comme critère et refuse un ordre d’ID invalide avant test/écriture.

## Vérifications

Environnement du test processus : Linux `7.0.0-1019-nvidia-aarch64`, `aarch64-unknown-linux-gnu`, rustc/Cargo 1.95.0, Bubblewrap 0.9.0. Les builds Cargo du runner utilisent cache préparé, `--offline --locked`, namespace réseau/PID séparées et runtime temporaire.

| Commande | Résultat |
|---|---|
| `cargo test --offline --locked --all-targets -- --test-threads=1` | Exit 0; 38 blocs, 725 réussis, 0 échec, 157 ignorés. Les tests ignorés ne sont pas exécutés par cette commande. Journal brut : [cargo-all-targets.log](cargo-all-targets.log). |
| `python3 scripts/run_forgejo_mission_delivery_process_e2e.py --only actual_mission_cli_admits_signed_webhook_after_closed_refusal_and_reuses_allocation` | Exit 0; admission processus isolée 1/1, reçu brut : [process-e2e.log](process-e2e.log). La fixture a compté 39 GET/API interactions. |
| `cargo test --offline --locked --test agent_runtime_contract untrusted_current_issue_context` | Exit 0; 1/1. |
| `cargo test --offline --locked --test mission_admission_contract admission_attaches_current_issue_text` | Exit 0; 1/1. |
| `cargo test --offline --locked --test mission_012_mcp_lifecycle complete_stdio_mcp_lifecycle_runs_red_write_green_shutdown_refusal_and_exit` | Exit 0; 1/1. Le test du processus MCP Rust vérifie un handoff contenant labels, leur exposition séparée et la conservation du document d’exigences. |
| `cargo test --offline --locked --test forgejo_mission_delivery_process help` | Exit 0; 1/1; l’aide versionnée expose les schémas et la source non fiable. |
| `cargo test --offline --locked --bin forgejo-mission-local current_issue_` | Exit 0; 2/2; labels triés, distinction inconnu/vide et formes invalides. |
| `python3 tests/test_mission_012_test_agent_adapter.py` | Exit 0; 11/11. Journal brut : [python-adapter-tests.log](python-adapter-tests.log). |
| `cargo fmt --all -- --check`, `git diff --check`, `python3 -m py_compile scripts/mission_012_test_agent_adapter.py tests/test_mission_012_test_agent_adapter.py` | Exit 0 après formatage Rust. |
| `python3 scripts/verify_cargo_license_state.py` | Exit 0; `license=null`, `license_file=null`, `publish=[]`, redistribution `not_assessed`, `NOASSERTION`; ce lot ne choisit pas de licence produit. |

Le premier test ciblé du parser avait échoué avant correction, car le résultat conservait le schéma legacy 1 au lieu de l’enveloppe 2 attendue lorsque la liste de labels est connue. Après correction, les tests ciblés, les deux coutures processus et la suite complète sont verts.

## Fichiers source et empreintes SHA-256

Ces empreintes lient les fichiers présents à cette révision de travail; le commit de base ne les contient pas, car plusieurs fichiers sont non suivis dans le worktree.

Le contrôle reproductible `sha256sum -c source-hashes.txt` retourne OK pour les onze chemins listés.

```text
9026887cb3fcf2e1e17236d517bbef7885be998a8d433316b214b9ace0e0f370  src/agent_runtime.rs
5c56b192b3c402c283a6777a3afd57db12e269da805cd4f84792b3a94d4cf6e1  src/mission_admission.rs
10df3ec892e051efdc539752427d82ac8b58b869e78d251e1971d449682e269e  src/bin/forgejo-mission-local.rs
60d0cd4a9bf2d2088479cee11bb4178bb58676087206b49ebdfcfb5de545350e  src/bin/mission-012-mcp-host.rs
9a4e7af83d7a5d05a3696489d8121a4e52aa000eef8014727cbae8bf4070ff61  tests/agent_runtime_contract.rs
9fc246cb7788a3a2718764da9ea9ac207cc52f18c8f9f30b36383fc72a4e602b  tests/mission_admission_contract.rs
8c3df0e88f3c90e794b732f1bf1ccd35f662c924b3a596582823991858274320  tests/mission_012_mcp_lifecycle.rs
1c1edfe99949f4fb19c8fd47f8ed1a3469eaa1e83237532fd5c85f60db4da8c8  tests/forgejo_mission_delivery_process.rs
6111f1d7e500fe8fa533506fe0cf07b93c5f9a4ee67b80e8fb783544222be0c8  tests/forgejo_mission_delivery_process_e2e.rs
ab77291dfcdf45125c33eb5847392090d2aa175ff21bd4e9359f176ac97d8914  scripts/mission_012_test_agent_adapter.py
81fc3c888da35455e98d7c1948f5698af5c86f906415528620164d98d572ec3c  tests/test_mission_012_test_agent_adapter.py
```

Le contrat des champs upstream repose sur la source Forgejo v17.0.0 au commit `0d791421f85ca13a63b665053d81ea765fbbc571`, audit CONTINUATION-329 : `modules/structs/issue_label.go::Label{ID,Name}` et `modules/structs/issue.go::Issue.Labels`. Aucun code Forgejo ni GitSpace n’est copié.

## Portée et limites

Le test processus et l’adaptateur sont des interfaces locales déterministes/fixtures. Ils ne démontrent ni une instance Forgejo live, ni AgentOS, ni l’exécution d’un LLM, ni que tout modèle résistera à l’injection. L’admission CLI, le cycle MCP host et l’adaptateur sont chacun vérifiés, mais les tests ne font pas parvenir le GET issue réel du fixture au processus agent MCP dans une seule exécution composée; ce chaînage reste à tester. Aucun claim de parcours LLM complet ne découle de ces tests. Cette tranche n’exécute pas une nouvelle PR, un verdict indépendant, une fusion ou une observation post-fusion; elle ne mesure ni tokens, coût, interventions, temps de livraison, exigences satisfaites ni régressions échappées. `725` tests passés ne sont pas un gain comparatif et `157` tests ignorés n’ont pas été exécutés. L’état général de mission, les licences/droits non décidés et le transfert GitLab PC4 restent ouverts; l’endpoint n’a pas été sondé durant ce lot.
