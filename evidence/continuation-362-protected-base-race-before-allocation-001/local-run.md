# CONTINUATION-362 — détecter une avance de base pendant l’inventaire

Date : 28 octobre 2026. Worktree `local/WEBHOOK-005-canonical-links`, base `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. Le dépôt était déjà fortement modifié; les changements de ce lot sont locaux, non commités et non publiés.

## Lacune reproduite et changement

`prepare_admission` lisait une seule fois le head protégé, puis lisait à ce commit l’arbre et le fichier d’exigences. Un déplacement de la branche pendant cet inventaire pouvait être invisible : le premier run rouge du vrai processus receiver → CLI a avancé `main` de `COMMIT` à `ADVANCED_COMMIT` après la réponse à la lecture de l’arbre, puis a observé `claimed` sur `COMMIT` et un handoff.

Le CLI relit maintenant `ProtectedBranchHeadV1` après validation des exigences et de leur arbitrage, avant `allocate_generation`. Le tuple complet `(branch, commit, protected)` doit être égal au snapshot initial; sinon le processus répond `inconclusive/mission_base_changed_during_admission`. Le test vérifie l’absence de réservation dans le journal et de fichier d’admission hôte. Le retry du même delivery relit l’arbre et le contexte au nouveau commit, puis réclame la génération 1.

Le test dédié est ajouté à `scripts/run_forgejo_mission_delivery_process_e2e.py`. Après la correction et le formatage, il passe 1/1 sous Bubblewrap `--unshare-net --unshare-pid`, avec fixture HTTP loopback. Les journaux rouge et vert consignent le comportement réel avant/après.

## Vérifications

- Rouge : `python3 scripts/run_forgejo_mission_delivery_process_e2e.py --only actual_mission_cli_refuses_branch_head_change_during_admission_then_retries_current_state` — échoue avant le fix parce que l’admission réclame le vieux commit (`base-race-red.log`).
- Vert après formatage : même commande — 1/1, reprise au commit avancé (`base-race-green-post-format.log`).
- Suite complète post-format : `cargo test --offline --locked --all-targets -- --test-threads=1` — code 0, 38 groupes, 739 réussis, 0 échec, 168 ignorés (`all-targets-post-format.log`).
- `cargo fmt --all -- --check`, `git diff --check` et `python3 -m py_compile scripts/run_forgejo_mission_delivery_process_e2e.py` — code 0.
- `python3 scripts/test_cargo_license_state.py` — 12/12; `verify_cargo_license_state.py` confirme `license=null`, `license_file=null`, `publish=[]`, `no_license_selected`, redistribution `not_assessed`, SPDX `NOASSERTION`.

Versions, source hashes, fichiers bruts, index local et preuve de préfixes append-only figurent dans ce dossier. Aucune dépendance ni décision de licence n’a été ajoutée.

## Portée et limites

Test de processus `n=1`, receiver et CLI réels dans un namespace réseau/PID privé, service Forgejo simulé par fixture locale; aucune instance Forgejo/GitLab live, aucun email ou secret, aucun LLM/AgentOS, PR, fusion ou benchmark. La valeur `COMMIT` n’est pas un commit upstream audité : c’est un identifiant synthétique de la fixture.

Le second GET n’est pas un CAS transactionnel. `main` peut avancer après ce contrôle et avant l’allocation; l’autorité de fusion doit toujours refaire ses contrôles à la fusion. Cette tranche réduit une fenêtre d’admission périmée sans revendiquer une garantie atomique. La suite complète a 168 tests ignorés et le test dédié est exécuté séparément.

## Prochaine action

Examiner l’ordre de validation du paquet de contexte et de l’allocation persistante : un contexte dont le commit source est déjà obsolète peut actuellement laisser une réservation récupérable avant de refuser le handoff. Vérifier si cette réservation est nécessaire pour la reprise après interruption; si non, déplacer uniquement les validations pures avant la mutation et préserver les preuves d’idempotence/reprise existantes.
