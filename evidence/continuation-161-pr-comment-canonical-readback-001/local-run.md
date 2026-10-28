# CONTINUATION-161 — readback canonique des commentaires de PR

Date : 26 octobre 2026. Worktree source : `.runtime/webhook-005-workspace`, HEAD de base `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0` (worktree fortement modifié; aucun commit créé).

## Changement observé

Le receiver reçoit l’événement Forgejo v17 `issue_comment`; un `pull_request` objet optionnel lie l’événement au parent PR et doit correspondre au numéro `issue.number` (avec `is_pull` cohérent). Le reconciler construit `PullRequestComment`, lit d’abord la PR courante et ensuite le commentaire par `GET /api/v1/repos/{owner}/{repo}/issues/comments/{comment_id}`. Il valide l’ID, `pull_request_url`, l’absence/valeur vide de `issue_url`, l’URL HTML ancrée à `#issuecomment-{id}`, la date, et restitue le corps API avec `body_trust=untrusted_user_content`. La sortie externe reste `resource.kind=pull_request` avec l’état du commentaire imbriqué. Le 404 ne signifie `absent` que pour un événement `deleted` après succès de la lecture du parent. L’admission de mission refuse l’événement commentaire avant les lectures d’exigences ou l’allocation.

Le readback PR et le readback commentaire sont deux GET séquentiels et non atomiques; aucun verdict de merge n’en est déduit. Le corps du commentaire reste une entrée hostile. La vérification couvre les fonctions Rust et le vrai processus receiver/CLI avec API loopback synthétique, pas une instance Forgejo live.

## Vérifications fraîches

- `cargo fmt --all -- --check` : succès après application de `cargo fmt --all`. Le premier format check a signalé uniquement la mise en ligne d’une assertion dans le nouveau test; aucune autre correction de style n’a été nécessaire.
- `cargo test --offline --locked --test webhook_reconcile_contract` : 59 réussis, 0 échec.
- `cargo test --offline --locked --test webhook_handler_contract` : 27 réussis, 0 échec.
- `cargo test --offline --locked --test webhook_mission_admission_contract pull_request_conversation_comment_cannot_create_mission_from_comment_text` : 1 réussi; le commentaire PR ne peut pas ouvrir une mission, aucune requête API d’admission, lecture d’exigences ou entrée de journal de mission.
- `python3 scripts/run_webhook_reconcile_cli_e2e.py --scenario actual_reconcile_cli_process_routes_issue_comment_pr_conversation_to_pull_request_api` : 1 réussi sous Bubblewrap avec `--unshare-net --unshare-pid`. La fixture observe exactement deux GET dans l’ordre attendu, sans Authorization; les champs PR courants et le corps du commentaire API sont présents, tandis que la sortie conserve l’étiquette de confiance non fiable. Le webhook signé reçu est synthétique et les deux réponses API viennent du fixture loopback.
- `python3 scripts/verify_cargo_license_state.py` : `status=verified`, `license=null`, `license_file=null`, `publish=[]`, `license_decision=no_license_selected`, `redistribution_rights=not_assessed`, `spdx_package_license_declared=NOASSERTION`.
- `cargo metadata --locked --offline --no-deps --format-version 1` : exécuté et conservé dans `cargo-metadata.json`; le champ package `license` demeure `null` et `license_file` demeure `null`.
- `git diff --check` : exit 0 sur le diff suivi. Le statut montre aussi des fichiers ciblés non suivis (les nouveaux contrats admission/processus et la matrice); ils ne sont pas couverts par Git. Un contrôle distinct des espaces finaux sur les 12 fichiers ciblés passe avec zéro ligne fautive (`targeted-whitespace-check.txt`). Aucune suite Rust globale n’a été relancée.

L’itération de développement précédente a d’abord obtenu un échec de contrat faute de commentaire courant, puis un échec du test processus qui révélait des champs de snapshot PR absents sur ce nouveau chemin; le helper commun de validation PR a été réutilisé et les assertions ciblées passent maintenant. Les sorties brutes de ces premières itérations ne sont pas incluses dans ce reçu; seules les sorties vertes ci-dessus sont des runs frais archivés ici.

## Source Forgejo examinée

Scratch `/tmp/forgejo-v17-base-cas-poc`, origin `https://codeberg.org/forgejo/forgejo.git`, HEAD exact `0d791421f85ca13a63b665053d81ea765fbbc571`; `git describe --tags --exact-match HEAD` retourne `v17.0.0` (reçu `upstream-tag-check.txt`). Sources ciblées : route API `routers/api/v1/api.go`; `repo.GetIssueComment` dans `routers/api/v1/repo/issue_comment.go`; `services/convert/issue_comment.go::ToAPIComment`; `models/issues/comment.go::{IssueURL,PRURL,HTMLURL}`; DTO `modules/structs/issue_comment.go::Comment`. `upstream-source-check.log` conserve les Git blob IDs exacts et la vérification `git diff --quiet HEAD --` réussie pour ces fichiers, malgré le scratch global modifié. Cette inspection établit le contrat de route et de sérialisation consulté, pas une exécution de cette route amont ni un audit complet de Forgejo. Aucun code Forgejo n’est copié ou adopté.

## Interfaces de runtime et portée

La recherche `rg -n 'impl.*AgentRuntimeV1|AgentRuntimeV1' src tests` ne trouve que le trait de contrat de production `src/agent_runtime.rs` et `tests/agent_runtime_contract.rs::FixtureRuntime`. Aucun provider LLM, Hermes, AgentOS ou OPSYS n’est invoqué dans ce lot; aucun coût ni donnée externe. Le reçu d’interface et les anciens audits locaux restent distincts de cette preuve de réconciliation.

Ce lot prouve uniquement un changement d’adaptateur, des contrats ciblés, un refus d’admission et un scénario local composé. Il ne prouve pas un readback live, une isolation de production, un parcours LLM, une intégration AgentOS, un gain comparatif ou l’achèvement de la mission.
