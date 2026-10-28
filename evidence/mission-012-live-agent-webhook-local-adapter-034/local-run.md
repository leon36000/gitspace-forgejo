# Run local 034 — notification de revue PR intégrée à la chaîne

Date : 24 octobre 2026. Worktree examiné : HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`, avec de nombreux changements locaux antérieurs. Forgejo `17.0.0+gitea-1.22.0`, binaire `/tmp/forgejo-17.0.0`, SHA-256 `e5ec437cd6357e8d1b4580cb47ab1daa58a32b33971a63db63446a535d33adb7`. Le clone officiel Forgejo v17 relu pour les contrats d’événements est au commit `0d791421f85ca13a63b665053d81ea765fbbc571`. Les sources exactes consommées par le runner et les binaires consommés sont hashés dans `composed-run.json` et les reçus de build; le build du binaire depuis le clone source n’est pas établi.

Commande reproductible :

```sh
python3 scripts/run_mission_012_live_composed_e2e.py \
  --forgejo-binary /tmp/forgejo-17.0.0 \
  --state-dir /tmp/mission012-composed-adapter-034 \
  --evidence-dir "$PWD/evidence/mission-012-live-agent-webhook-local-adapter-034" \
  --execution-id live-mcp-034 \
  --allocation-id mission012-local-adapter-034 \
  --budget-cap-usd 0.20 \
  --agent-adapter local-test-adapter
```

## Résultat observé

- Une livraison Forgejo réelle `issues/opened` a créé le claim versionné pour `MISSION-012-CLAMP-001`, hash `3a3c444075c93a3bcfb2bec17d1a2e74ec0051effc2994e6b02ac095eb0dd431`, base `6ac1aabc0772d73f1f2fa9fb900be90c5f499290`.
- L’adaptateur `local-test-adapter` a consommé le contexte et les outils locaux. La première vérification d’acceptation a échoué à la borne supérieure du clamp; le changement limité à `src/lib.rs` a ensuite passé les deux critères. Aucun modèle n’a été appelé (`model_invoked_during_run=false`, coût rapporté 0 USD).
- La PR locale #2 a émis trois notifications reçues avec HTTP 202 : `pull_request/opened`, `pull_request_approved/reviewed`, puis `pull_request/closed`. La revue approuvée est liée au PR #2, au commit candidat `cb10850e71c7711ab9b4a5f728eff3c98025af67` et à la base admise. Forgejo a enregistré trois tâches et leurs UUID sont liés aux trois livraisons sans supposer leur ordre d’arrivée.
- La notification de revue sert seulement de déclencheur de réconciliation, jamais de preuve d’approbation. L’approbation a été relue via l’API Forgejo et liée au candidat exact; l’autorité Rust distincte a ensuite fusionné et relu le commit `1956d1baa5babd9790ae75fcda65831447f6ccea`. Les trois deliveries ont été réconciliées vers l’état courant `closed/merged=true`, tête candidat et base commit de fusion.
- Au nettoyage, Forgejo confirme le hook désactivé, les tâches et reçus ne contiennent pas le secret, le récepteur s’est arrêté et son listener a disparu. Les résultats détaillés sont dans `pr-webhook-reconciliation.json`, `forgejo-result.json`, `independent-verifier.json` et les reçus d’admission du dossier voisin `../mission-012-live-agent-webhook-local-adapter-034-admission/`.

## Vérification locale

- `cargo fmt --all -- --check`, `cargo test --offline --locked` — 387 réussis, 0 échec, 14 ignorés; les ignorés sont des helpers annotés comme nécessitant une fixture réseau isolée. Clippy passe avec les trois allowances historiques `too_many_arguments`, `bool_assert_comparison` et `manual_div_ceil`.
- `python3 -m py_compile` sur les runners et tests touchés, puis `python3 -m unittest tests.test_forgejo_emitted_webhook_e2e tests.test_mission_012_live_composed_e2e tests.test_mission_012_live_agent_pr_e2e -v` — 62 tests réussis.
- `git diff --check` et les deux manifestes `SHA256SUMS` de ce run et de l’admission — succès après finalisation. Sorties conservées dans `cargo-test.log`, `python-tests.log`, `clippy.log`, `rustfmt.log`, `pycompile.log` et `diff-check.log`.

## Portée et limites

Le parcours PR/revue/fusion a été observé avec le binaire Forgejo local et l’adaptateur déterministe; aucun LLM, AgentOS, Forge Code, RAG ou moteur de mémoire n’a participé. C’est un cas (`n=1`), sans comparaison appariée Forgejo+CI conventionnelle; il ne prouve aucun gain général en réussite, temps, interventions, tokens ou coût. Les processus locaux partagent encore l’UID, et le verrou de fusion ne protège que les writers coopératifs du même hôte; Forgejo ne fournit pas de CAS de base contre un writer externe. Le hash de binaire identifie le fixture, mais sa provenance de build depuis le clone source n’est pas établie. Cargo `license` reste `null`; aucun droit de réutilisation de GitSpace ou de redistribution n’est ajouté par ce lot.
