# CONTINUATION-348 — Forgejo issue `reopened` live

Date: 28 octobre 2026. Worktree: `/home/pc6/Documents/Codex/2026-10-21-mission-construire-une-forge-con-ue/.runtime/webhook-005-workspace`; branch `local/WEBHOOK-005-canonical-links`; base HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. Les fichiers runner/tests étaient non suivis avant ce lot et le restent; aucun commit ou transfert distant n’a été effectué.

## Changement

Le parseur de notifications d’issues accepte maintenant l’action native `reopened`; l’interface expose `--reopen-issue`. Après avoir créé, fermé et édité l’issue jetable, le runner la rouvre avec `PATCH /repos/{owner}/{repo}/issues/{index}`, exige HTTP 201 et un GET API `state=open`, puis attend exactement une livraison `issues/reopened`. Il vérifie la séquence croissante et un UUID distinct, ajoute cet événement à la liaison des tâches et le réconcilie par une lecture API courante. Les anciennes ouvertures, fermetures et éditions doivent aussi refléter l’état final courant. Les compteurs et offsets PR incluent la réouverture d’issue. L’exécution combinée avec `--change-issue-labels` teste ces offsets avec deux livraisons supplémentaires.

Référence amont ciblée déjà consignée dans `docs/source-inventory.md`, section CONTINUATION-300: Forgejo officiel v17.0.0, commit `0d791421f85ca13a63b665053d81ea765fbbc571`; `modules/structs/hook.go::HookIssueReOpened` fournit `reopened` et `services/webhook/notifier.go::IssueChangeStatus` l’émet pour l’issue sous le header `issues`. Il s’agit d’une lecture source ciblée, pas d’un audit complet du dépôt upstream.

## Vérifications

- RED: `PYTHONPATH=tests python3 -m unittest -v test_forgejo_emitted_webhook_e2e.EmittedWebhookRunnerCliTests.test_exposes_live_issue_reopen_scenario test_forgejo_emitted_webhook_e2e.ParseLiveDeliveryRecordTests.test_extracts_native_issue_reopened_action` échoue d’abord pour les raisons prévues: flag absent et parseur qui rejette l’action.
- GREEN ciblé: les deux mêmes tests passent.
- Suite Python entière du module: `PYTHONPATH=tests python3 -m unittest -v test_forgejo_emitted_webhook_e2e` — 145 tests, tous réussis, Python 3.12.3.
- `python3 -m py_compile scripts/run_forgejo_live_webhook_reconcile_emitted.py tests/test_forgejo_emitted_webhook_e2e.py` — code retour 0.
- `pytest` n’est pas présent (`python3 -m pytest` retourne « No module named pytest »); aucun paquet n’a été installé; les tests du module utilisent `unittest` standard.
- Instance locale jetable: `python3 scripts/run_forgejo_live_webhook_reconcile_emitted.py --forgejo-binary /tmp/forgejo-17.0.0 --repo-root /home/pc6/Documents/Codex/2026-10-21-mission-construire-une-forge-con-ue/.runtime/webhook-005-workspace --state-dir /tmp/forgejo-cont-348-issue-reopen-state-001 --evidence-dir /home/pc6/Documents/Codex/2026-10-21-mission-construire-une-forge-con-ue/.runtime/webhook-005-workspace/evidence/continuation-348-issue-reopen-event-live-001/forgejo-run-001 --change-issue-labels --reopen-issue` — code retour 0, verdict `passed`.
- Depuis `forgejo-run-001/`, `sha256sum -c SHA256SUMS` — 19/19 entrées `OK`.

Le binaire déclare `forgejo version 17.0.0+gitea-1.22.0`, Go 1.26.9; SHA-256 `e5ec437cd6357e8d1b4580cb47ab1daa58a32b33971a63db63446a535d33adb7`. Son commit de compilation n’est pas établi. Le run reçoit dix livraisons et valide dix tâches Forgejo HTTP 202. Séquences 2–3: labels; 4: fermeture; 5–6: éditions; 7: `issues/reopened`; 8–10: ouverture PR, commentaire de conversation et fermeture PR. Pour la séquence 7, le readback API est `open`; la réconciliation donne `verdict=current_state_read`, un appel upstream, ressource `issue #1`, état canonique `open`. Les réconciliations des événements issue plus anciens lisent aussi l’état courant `open`. Le rejeu doublon reste idempotent et le journal ne change pas. Readback de fin: hook inactif, processus terminés, listeners Forgejo/receiver absents.

## Fichiers et empreintes

- Runner: `scripts/run_forgejo_live_webhook_reconcile_emitted.py`, SHA-256 `98c6a588c707db037a7018035eca63bb2915c8ee658ef4cb811f8ab17cccaa36`.
- Tests: `tests/test_forgejo_emitted_webhook_e2e.py`, SHA-256 `536559812b7047926c14b13a7b84b6d990cc37b3680bdc106f887a6054962b45`.
- Résultat live: `forgejo-run-001/result.json`, SHA-256 `3ce126bfba55d2e9f92d45186cced7478c97eb7011bfef2c4be79eddf976ec97`.
- Manifest du run: `forgejo-run-001/SHA256SUMS`, SHA-256 `e648a7e14d00b9107cb1ad566d764f13284bffaf733af045fe24fc456d049191`.
- Logs tests RED/GREEN/suite: `issue-reopen-python-red.log` `5593f0fb206d4b008f1b8719ed4d1ec7e82833398d3402f27aeb9a182448005c`; `issue-reopen-python-green-targeted.log` `1df76a6cc2d45ff2c9a4c83990d22787b78ebd8199fcf55773be00ca8dcb77f8`; `issue-reopen-python-full-suite.log` `fe0458111f7e8ebfec73571f5d80acc706048f6282c33179b62535a1ea0ba725`.

## Limites

Un seul run live (`n=1`) sur Forgejo local avec données privées jetables. Il prouve l’émission et la réconciliation de cette transition et l’ajustement des offsets dans cette composition. Il ne prouve ni intégration GitLab, ni LLM/AgentOS, ni changement de code fusionné, ni réduction de coûts ou temps, ni amélioration générale. Aucun Rust n’a changé et la suite Rust n’a pas été relancée. La mission et le worktree restent incomplets et non commités.
