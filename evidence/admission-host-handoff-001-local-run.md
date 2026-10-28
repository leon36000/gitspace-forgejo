# ADMISSION-HOST-HANDOFF-001 — transfert typé vers l’hôte MCP

Date : 2026-10-23. Worktree local `local/WEBHOOK-005-canonical-links`, HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`; fichiers sources et preuves non commités. Toolchain : rustc 1.95.0 (59807616e 2026-04-14), Cargo 1.95.0 (f2d3ce0bd 2026-03-21).

## Changement observable

- `MissionHostAdmissionV1` est construit depuis le `MissionAdmissionReceiptV2` retourné par `admit_and_claim_mission_v2`. Le DTO versionné transporte scope, issue, commit Git, chemin/hash d’exigence, allocation durable et requête exacte. Sa validation contrôle les égalités redondantes, le hash des exigences, le commit de workspace, l’ID de l’exigence et les invariants de scope/requête. Le DTO n’expose ni résultat `success` ni autorité de fusion.
- Le processus `mission-012-mcp-host` consomme désormais ce DTO dans la config hôte schéma 3. Les champs scope/issue/génération/allocation/requête ne sont plus recopiés en parallèle. L’hôte revalide le profil fixé, puis utilise le journal de claims durable pour la lease; une contradiction de commit est refusée avant lease et avant toute réponse MCP.
- Le contrat de cycle de vie compose la vraie fonction d’admission de la bibliothèque avec le vrai processus hôte via ce DTO. La source Git/issue/exigence est synthétique et locale; le harnais écrit le fichier de config hôte. Le CLI `forgejo-mission-local` n’émet pas encore le DTO et aucun événement webhook n’est transporté jusqu’au processus MCP dans cette tranche.

## Preuves et vérifications

- Rouge avant implémentation : [admission-host-handoff-001-red.log](admission-host-handoff-001-red.log), le contrat échoue parce que l’ancien hôte attend la config 2 et aucun `admission` versionné n’existe.
- Ciblé : `cargo test --offline --locked --test mission_012_mcp_lifecycle -- --nocapture` — 22 réussis, 0 échec; [journal](admission-host-handoff-001-focused.log). Cas ajoutés : forme v1 sans champs copiés et refus d’un `base_commit` qui contredit la requête, sans lease ni trame MCP. Les contrats existants vérifient aussi tree incohérent, SHA périmé/inconnu, claim absent, génération dépassée, chemin d’état interdit et cycle stdio complet.
- Suite complète : `cargo test --offline --locked --all-targets` — 29 suites, 293 réussis, 0 échec, 9 ignorés; [journal](admission-host-handoff-001-all-targets.log). Les ignorés sont des scénarios séparés nécessitant le runner Bubblewrap décrit ailleurs; ils ne sont pas comptés comme réussites ici.
- `cargo clippy --offline --locked --all-targets -- -D warnings -A clippy::too_many_arguments -A clippy::bool_assert_comparison -A clippy::manual_div_ceil` — code 0 avec trois catégories de lints préexistantes neutralisées; [journal](admission-host-handoff-001-clippy.log).
- `cargo build --offline --locked --release --all-targets`, `cargo fmt --all -- --check` et `git diff --check` — code 0; [build](admission-host-handoff-001-release-build.log), [rustfmt](admission-host-handoff-001-rustfmt.log), [diff-check](admission-host-handoff-001-diff-check.log). Contrôle additionnel LF/espaces sur les quatre fichiers Rust touchés : 0 erreur.
- L’état de licence a été revalidé dans ce worktree : les 5 tests Python passent; `verify_cargo_license_state.py` rend `license=null`, `license_file=null`, `publish=[]`, `license_decision=unselected`, `redistribution_rights=not_assessed`; journaux [tests](cargo-license-state-003-tests.log) et [readback](cargo-license-state-003-verify.log). Aucun choix de licence ou fichier LICENSE n’est ajouté.

## Limites

Ce résultat est une intégration de bibliothèque et de processus MCP avec fixture locale, pas un parcours Forgejo live ni webhook→CLI→MCP. Il n’exerce aucun LLM, AgentOS, RAG ou moteur de mémoire. Le profil MCP reste explicitement épinglé à MISSION-012; le DTO n’en fait pas un host générique. La provenance du DTO est vérifiée contre les invariants de profil et le journal de claim, mais il n’est pas signé; un processus hostile du même UID peut modifier les fichiers d’état et l’isolation par worktree ne défend pas cette frontière. La tête protégée reste un instantané au démarrage et ne prouve pas la fraîcheur à la fusion. Le lot ne constitue pas une preuve comparative de qualité, coût ou délai.

La revue statique Claude Sonnet 5.5 du lot ADMISSION-MCP-001 est toujours suivie séparément et ne doit pas être attribuée à cette tranche. Une revue Codex lecture seule du nouveau DTO est demandée; elle n’est pas encore reçue au moment de ce rapport.
