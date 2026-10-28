# ADMISSION-MCP-001 — admission réelle vers l’hôte MCP, base liée au SHA Git admis

Date : 2026-10-23 (session Ralph `eaa233ab…`, itérations 1–2)
Checkout : `local/WEBHOOK-005-canonical-links`, HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`, travail local non commité.
Fichiers de code modifiés dans ce lot : `src/mission_012_host.rs`, `src/bin/mission-012-mcp-host.rs`, `tests/mission_012_mcp_lifecycle.rs` (tous trois non suivis par Git avant ce lot). Aucun autre fichier source, aucune dépendance, aucun outil installé.

## Écart traité

Avant ce lot, l’hôte `mission-012-mcp-host` épinglait la chaîne `2b230ecda8d57dabbb36eee1399efbe4c8c875ad` comme base, sans jamais comparer `base_dir` à un objet Git ; aucun dépôt local ne contenait ce commit. Le test de cycle de vie fabriquait lui-même allocation et claim (`allocate_generation` + `claim_allocated`) au lieu de consommer la sortie de l’admission.

Rouge observé (`evidence/mission-012-admission-mcp-sha-binding-red.log`) : avec un fichier absent de tout commit dans `base` et `candidate`, l’hôte acquiert un lease (`left: 1, right: 0`) et expose le workspace. Le cycle de vie complet existant passait (`evidence/mission-012-admission-mcp-existing-lifecycle.log`).

## Changement

1. **Composition par la fonction d’admission réelle.** `tests/mission_012_mcp_lifecycle.rs::claim_execution_from` appelle la fonction publique `admit_and_claim_mission_v2` avec une source synthétique déterministe `SyntheticAdmissionSource` adossée à un dépôt Git local (issue ouverte, tête protégée `refs/heads/main`, exigences et inventaire lus par `git show`/`git ls-tree` au commit exact). Requête, `mission_id`, génération, `allocation_id`, commit de base, octets d’exigence et claim durable sont la sortie de l’admission ; le harnais ne fournit que le binding hôte, l’ID de workspace, les capacités épinglées et le paquet de contexte. L’hôte reçoit exactement `receipt.request` et le même journal.
2. **Liaison du workspace au SHA admis avant tout lease ou échange MCP.** `src/mission_012_host.rs::bind_workspace_to_admitted_commit` exige un SHA complet de 40 hex minuscules, le résout comme commit dans le dépôt Git hôte (`rev-parse --verify --quiet <sha>^{commit}` doit se rendre lui-même), exige qu’il soit encore la tête de `refs/heads/<branche épinglée>`, puis compare blob par blob `base_dir` **et** le candidat initial à l’arbre du commit (`git_commit_matches_snapshot` existant : même ensemble de chemins, modes, OID de blob, fichiers réguliers mono-lien, sans lien symbolique ni `.git`). Le résultat (`commit`, `tree`, `branch`, empreinte du manifeste) entre dans `execution_binding` de chaque reçu.
3. **Configuration hôte v2.** `workspace.trusted_git_dir` est obligatoire et absolu ; un dépôt qui contient un répertoire du sandbox ou s’y trouve est refusé (`host_state_path_inside_workspace`). L’épinglage de la chaîne de commit est retiré : le commit vient de la requête claimée et n’est accepté qu’après la preuve Git. Restent épinglés : origine/propriétaire/dépôt/branche, issue 1, octets et révision d’exigence, ID de workspace, capacités et budget, politique de vérification (constantes du module). L’hôte vérifie aussi que `requirements/MISSION-012-CLAMP-001.json` dans la base liée est l’octet près l’exigence épinglée, et que le snapshot de base repris par `CandidateToolHost::new` a l’empreinte qui vient d’être liée. Une configuration v1 est refusée.

Ordre dans `main` : lecture/validation de la config et du profil → chemins d’état → reçu absent → **liaison Git base+candidat** → exigence dans la base → construction du `CandidateToolHost` (contrôle d’empreinte) → `acquire_lease` (le hash de requête doit égaler le claim durable) → création du journal de reçus → première lecture stdin. Aucune trame MCP n’est lue ni répondue avant la liaison.

## Couverture

`tests/mission_012_mcp_lifecycle.rs` (20 tests, dont 7 nouveaux) :

| Test | Refus attendu, avant lease, sans reçu ni réponse MCP |
|---|---|
| `mcp_host_refuses_a_base_dir_that_is_not_the_admitted_git_tree` (ancien rouge) | `workspace_base_tree_mismatch` |
| `mcp_host_refuses_base_and_candidate_left_at_an_older_tree` | `workspace_base_tree_mismatch` (branche avancée avant admission, workspace resté à l’ancien arbre) |
| `mcp_host_refuses_a_stale_admitted_commit_after_the_protected_branch_advances` | `workspace_base_commit_stale` |
| `mcp_host_refuses_an_admitted_commit_missing_from_the_trusted_repository` | `workspace_base_commit_unknown` (même arbre, autre SHA : l’égalité de contenu ne remplace pas le commit admis) |
| `mcp_host_refuses_an_admitted_request_without_its_durable_claim` | `execution_allocation_missing` (journal sans le claim d’admission) |
| `mcp_host_refuses_a_trusted_repository_inside_the_workspace` | `host_state_path_inside_workspace` (candidat, cible) |
| `mcp_host_refuses_a_version_1_config_without_a_trusted_repository` | `host_config_invalid` |

Chaque refus envoie aussi `initialize` puis une lecture de `src/lib.rs` : stdout reste vide, donc aucun contexte n’atteint le client. Le cycle complet vérifie que `base_commit` du reçu égale la sortie d’admission et la tête du dépôt de confiance, que `base_tree` égale `rev-parse <sha>^{tree}`, et que le journal contient exactement une allocation, un claim dont `request_sha256` égale la requête d’admission et le reçu, et un lease. `mcp_host_refuses_an_unready_candidate_without_burning_the_lease` refuse maintenant par `candidate_not_at_admitted_base` puis réussit après réparation avec le même ID de lease. Le test unitaire `base_binding_requires_a_full_lowercase_commit_and_a_plain_branch` refuse SHA court, majuscule, non hex, 64 hex et `HEAD`, ainsi que des noms de branche invalides.

## Vérification

Environnement et référence : `rustc 1.95.0 (59807616e 2026-04-14)`, `cargo 1.95.0 (f2d3ce0bd 2026-03-21)`, HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. Les trois fichiers Rust du lot sont non suivis dans cet état Git; les journaux ci-dessous ont été générés hors ligne dans ce checkout après leur formatage final.

- Rouge avant correction : [sha-binding-red](mission-012-admission-mcp-sha-binding-red.log) montre l’acquisition de lease sur un tree non admis. Le test de cycle antérieur passe dans [existing-lifecycle](mission-012-admission-mcp-existing-lifecycle.log).
- Focalisé après correction : `cargo test --offline --locked --test mission_012_mcp_lifecycle -- --nocapture` — 20 réussis, 0 échec : [journal final](mission-012-admission-mcp-focused-independent-final.log).
- Suite `cargo test --offline --locked --all-targets` — 29 suites, 291 réussis, 0 échec, 9 ignorés : [journal final](mission-012-admission-mcp-all-targets-independent-final.log). Les tests ignorés sont des scénarios de processus nécessitant la commande d’isolation documentée par ces tests; ils ne sont pas comptés comme réussites.
- Clippy all-targets avec `-D warnings` et les trois exceptions historiques du dépôt (`too_many_arguments`, `bool_assert_comparison`, `manual_div_ceil`) : [journal final](mission-012-admission-mcp-clippy-independent-final.log), sortie 0.
- `cargo build --offline --locked --release --all-targets` : [journal final](mission-012-admission-mcp-release-build-independent-final.log), sortie 0.
- `cargo fmt --all -- --check` : [journal final](mission-012-admission-mcp-rustfmt-independent-final.log), sortie 0 après formatage ciblé des trois fichiers du lot.
- `git diff --check` : [journal final](mission-012-admission-mcp-diff-check-independent-final.log), sortie 0. Un contrôle séparé des espaces en fin de ligne sur les trois fichiers Rust et ce rapport a trouvé 0 occurrence.

Les empreintes SHA-256 des trois fichiers Rust et des journaux rouge/vert finaux sont consignées dans [le manifeste](mission-012-admission-mcp-final-sha256.txt).

Ces résultats confirment le test de contrat/processus local et les contrôles du workspace. Ils ne démontrent ni une admission Forgejo live, ni le transport d’un reçu par le CLI d’admission, ni un agent LLM dans ce parcours. La revue Sonnet demandée est suivie séparément; elle ne sera pas qualifiée de terminée tant que son constat et la disposition des éventuels findings ne sont pas enregistrés.

## Revue statique indépendante

Un sous-agent Codex a relu les trois fichiers du binding en lecture seule. Il ne relève pas de nouveau défaut matériel dans le lien admission→claim ni dans le refus avant lease et MCP. Il confirme que la source de test est synthétique et que le test écrit encore la configuration hôte; le CLI d’admission n’est pas lancé. Sa limite matérielle est que la tête de branche n’est contrôlée qu’au démarrage du host : l’avance est testée avant le démarrage, pas après le binding pendant l’exécution ou au moment d’une fusion. Cette tranche ne comporte aucun chemin de fusion; le vérificateur/autorité de fusion devra relire la base courante et invalider la preuve si elle a changé. Voir [revue statique Codex](mission-012-admission-mcp-codex-review.md). Cette revue n’est pas la revue Sonnet 5.5 demandée à Claude Code; au moment de la rédaction, aucun compte rendu Sonnet n’est enregistré.

## Limites

- **Pas d’E2E webhook→MCP.** La composition passe par la fonction `admit_and_claim_mission_v2` dans le harnais, pas par le processus `forgejo-mission-local`. Ce CLI reste épinglé à `127.0.0.1:3130` / `agent-forge-core` / issue 41 / `REQ-001` avec d’autres capacités ; sa sortie ne peut pas alimenter ce profil MISSION-012 et aucun binaire ne transporte un reçu d’admission vers l’hôte. La configuration hôte reste écrite par l’hôte de confiance ; son intégrité repose sur le claim durable (hash de requête) et la preuve Git.
- **Source synthétique.** La source d’admission est un dépôt Git local, pas Forgejo ; aucune lecture réseau, aucune authentification ni lecture de règle de protection.
- **Instantané, même UID.** La liaison vaut au moment du contrôle : un processus du même UID peut modifier `base_dir`, le candidat, le dépôt de confiance ou le journal ensuite. Le dépôt de confiance est lu avec la configuration Git globale/système neutralisée, mais sa configuration locale reste celle de l’hôte.
- **Fraîcheur limitée au démarrage.** La tête de branche est lue une fois avant le lease; le host ne la relit pas à chaque outil ou à la finalisation. La preuve ne peut donc pas autoriser une fusion sans un contrôle distinct de la base courante lié au candidat exact.
- **SHA-1 uniquement.** Les dépôts SHA-256 (OID de 64 hex) sont refusés par la liaison, bien que l’admission les accepte.
- Les limites déjà listées en tête du module (annulation en vol, finalisation occupée, reçu après effet, lease non relâché) restent inchangées.
