# CONTINUATION-338 — projection de l’état courant `draft` des PR

Date : 28 octobre 2026 (machine America/Montreal). Travail local dans `.runtime/webhook-005-workspace`, HEAD de base `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`, branche `local/WEBHOOK-005-canonical-links`; dépôt très modifié avant ce lot. Aucun commit, reset, nettoyage, push ou modification distante.

## Lacune et correction

`PullRequestSnapshotV1` utilisait déjà le booléen `draft` pour le préflight de fusion, mais la projection webhook `ReconciledState` ne l’exposait pas. Une lecture de PR ne permettait donc pas au consommateur d’identifier le brouillon : le JSON renvoyait `canonical_state.draft=null`. Le test rouge `pull_request_draft_projection_uses_current_api_not_webhook_snapshot` a reproduit `null` alors que la réponse API courante était `false` et le snapshot webhook était `true`.

Le DTO contient maintenant `draft: Option<bool>`. Pour une PR ou un commentaire de PR, `current_pull_request_fields` exige le booléen de la lecture API courante; les événements ne fournissent jamais ce champ canonique. Issue, branche et workflow conservent `null`. L’absence, `null`, chaîne et nombre donnent `InvalidCurrentStateField("draft")`, sans état canonique. Les tests couvrent les deux contradictions event/API et les quatre formes invalides; le test du commentaire vérifie que la PR parente courante fournit son propre `draft`.

La sortie single est maintenant schema 16 (ancien 15), batch schema 20 (ancien 19), capabilities v23 (ancien v22). Les gardes consommateur dans Rust/Python ont été migrées. Capabilities publie source `current_forgejo_pull_request_api`, fraîcheur `current_api_read`, et `merge_authorizing=false`. Le champ reste descriptif; il ne remplace ni le snapshot de préflight merge, ni des checks de protection, ni le verrouillage atomique de la fusion.

## Référence amont et droits

Le dépôt scratch Forgejo est `/tmp/forgejo-v17-base-cas-poc`, HEAD `0d791421f85ca13a63b665053d81ea765fbbc571` (Forgejo officiel v17.0.0). Lecture non mutante de `modules/structs/pull.go:25` : `Draft bool json:"draft"`; hash SHA-256 du blob `1472f689d9e80da512cc8edafa5d647daafccdf060dfba0d4a74d3be5cfe06e3`. `LICENSE` est la GNU GPL version 3 ou ultérieure (sections 638–645), hash du blob `3972dc9744f6499f0f9b2dbf76696f2ae7ad8af9b23dde66d6af86c9dfb36986`. Ce travail ne copie aucun code Forgejo. Le scratch upstream est déjà dirty et n’a pas été modifié dans ce lot.

Aucune dépendance n’a été ajoutée. Garde Cargo observée : `license=null`, `license_file=null`, `publish=[]`, `no_license_selected`, droits de redistribution `not_assessed`, marqueur export SPDX `NOASSERTION`. Ce marqueur n’est pas un choix de licence et aucun avis juridique n’est donné.

## Vérifications

| Vérification | Résultat |
|---|---:|
| Test rouge initial `pull_request_draft_projection_uses_current_api_not_webhook_snapshot` | échec attendu : gauche `null`, droite `false` |
| `cargo test --offline --locked --test webhook_reconcile_contract -- --test-threads=1` | 87 réussis |
| `cargo test --offline --locked --all-targets -- --test-threads=1` | exit 0, tous les groupes terminés sans échec; les tests explicitement ignorés restent réservés à leurs runners dédiés |
| `tests/test_forgejo_emitted_webhook_e2e.py` | 131 réussis |
| `tests/test_forgejo_webhook_native_replay_e2e.py` | 30 réussis |
| `tests/test_forgejo_live_org_team_reviewer_readback.py` | 29 réussis |
| `python3 scripts/test_cargo_license_state.py` | 12 réussis |
| `scripts/verify_cargo_license_state.py` | `metadata_policy_verified`, aucun choix de licence |
| `cargo fmt --all -- --check` | passe |
| `git diff --check` | passe |

Deux exécutions all-targets antérieures ont d’abord détecté un fixture PR qui manquait `draft`, puis une assertion single/batch qui attendait encore les anciens numéros de schéma. Les fixtures et assertions ont été corrigés; la dernière exécution complète a retourné 0. Le reçu enregistre ce diagnostic pour que le premier échec ne soit pas masqué.

Toolchain : Cargo 1.95.0, rustc 1.95.0, Python 3.12.3; détails/hashes dans `tool-versions.txt` et `source-hashes.txt`. Les sorties ciblées sont stockées dans les fichiers `*-tests.log`, `rustfmt.log`, `diff-check.log` et `cargo-license-state.json`. La sortie all-targets complète a été diffusée en plusieurs parties par l’outil; `all-targets-result.txt` consigne son code de sortie final plutôt que de la présenter comme un transcript brut.

## Portée et limites

Les nouveaux contrats utilisent des fixtures API locales; aucune instance Forgejo live n’a été utilisée pour ce lot. Les adaptations des validateurs sont couvertes par les trois modules Python cités. Aucun AgentOS, modèle, Claude, accès courriel, secret, dépôt GitLab, service externe, PR réelle ou fusion n’a été impliqué. Ce lot ne démontre ni parcours LLM, ni gain comparatif, ni autorité de fusion atomique. Les consommateurs externes au dépôt pour ces numéros de schéma n’ont pas été inventoriés.

La mission globale reste incomplète : comparer contre Forgejo+agent+CI conventionnels avec un runtime et budget autorisés, compléter les refus cumulatifs, et établir le parcours de livraison/validation post-fusion dans les conditions autorisées.
