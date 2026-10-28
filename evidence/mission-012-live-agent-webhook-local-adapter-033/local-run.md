# Run local 033 — admission webhook → PR → vérification → fusion

Date : 24 octobre 2026. Exécution sur le worktree local de la forge, HEAD source `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0` avec modifications locales. Forgejo `17.0.0+gitea-1.22.0`, binaire SHA-256 `e5ec437cd6357e8d1b4580cb47ab1daa58a32b33971a63db63446a535d33adb7`; le build du binaire depuis le tag n’est pas établi. L’empreinte de chacun des 12 fichiers source utilisés est dans [`composed-run.json`](composed-run.json); le résultat et les manifestes d’admission sont dans le dossier voisin `../mission-012-live-agent-webhook-local-adapter-033-admission/`.

Commande reproductible :

```sh
python3 scripts/run_mission_012_live_composed_e2e.py \
  --forgejo-binary /tmp/forgejo-17.0.0 \
  --state-dir /tmp/mission012-composed-adapter-033 \
  --evidence-dir "$PWD/evidence/mission-012-live-agent-webhook-local-adapter-033" \
  --execution-id live-mcp-033 \
  --allocation-id mission012-local-adapter-033 \
  --budget-cap-usd 0.20 \
  --agent-adapter local-test-adapter
```

## Résultat observé

- Une livraison Forgejo réelle `issues/opened` a déclenché l’admission de l’exigence versionnée `MISSION-012-CLAMP-001`, SHA-256 `3a3c444075c93a3bcfb2bec17d1a2e74ec0051effc2994e6b02ac095eb0dd431`, sur la base `c9324f31f7cad9702c4e61d8758de16e555ad190`. Le claim est lié à `mission012-local-adapter-033`; le rejeu ne crée pas une seconde admission et le credential invalide est refusé avant claim.
- L’adaptateur déterministe a consommé le contexte/outillage local et n’a appelé aucun modèle (`reported_cost_usd=0`). Le premier `cargo test --offline --locked` reproduit l’échec protégé de la borne supérieure; après correction, les deux tests d’acceptation passent. Le diff PR ne change que `src/lib.rs`, SHA-256 `173304ad7d32aabc1e8e1ee41eabda3398a8690798e68f906bcebc73cbcd6d15`.
- Forgejo a créé la PR #2. Le merge avant approbation est refusé HTTP 405; le reviewer non autorisé à fusionner est aussi refusé HTTP 405. Le vérifieur indépendant relance l’acceptance sur le commit candidat `cebedec3006be9c3624f279cd32d00d3bfa9a9c0` dans Bubblewrap avec réseau isolé, snapshot en lecture seule, Cargo hors ligne/verrouillé. L’autorité Rust distincte fusionne ensuite et relit l’état; commit de merge `2072c6b942e8b2867e352b5fd40abf22aace0ce0`.
- Deux notifications PR distinctes (`opened`, puis `closed/merged`) ont deux tâches Forgejo correspondantes, toutes deux HTTP 202. Les deux deliveries sont réconciliées vers l’état courant fermé/fusionné. Le SHA de base de l’ouverture correspond à la base admise; celui de l’événement fermé et du readback canonique correspond au commit de merge.
- Forgejo développe le sélecteur API `pull_request` en plusieurs sous-événements, dont `pull_request_review_approved`; le récepteur actuel refuse ce dernier. Le hook est donc désactivé avant la revue puis réactivé avant les contrôles prémerge, avec lecture API après chaque PATCH. La receipt v2 expose la période sans collecte : les notifications PR de cette fenêtre ne font pas partie de la preuve. Les lectures API et protections de branche sont encore revérifiées avant merge. Au nettoyage, le hook est inactif (`is_active=0` en SQLite), le récepteur a quitté avec code 0 et ni le port Forgejo 3132 ni le port du récepteur n’écoutent.

## Vérification

- `python3 -m py_compile scripts/run_mission_012_live_agent_pr_e2e.py scripts/run_mission_012_live_composed_e2e.py tests/test_mission_012_live_agent_pr_e2e.py tests/test_mission_012_live_composed_e2e.py` — réussite.
- `python3 -m unittest tests.test_mission_012_live_composed_e2e tests.test_mission_012_live_agent_pr_e2e tests.test_forgejo_live_webhook_mission_admission` — 63 tests, réussite.
- `cargo test --offline --locked --bin forgejo-reconcile-local` — 91 tests, réussite.
- `cargo fmt --all -- --check` et `git diff --check` — réussite.
- `sha256sum -c SHA256SUMS` dans les dossiers du run et de l’admission — tous les fichiers listés valides. Les sorties d’acceptance rouge/verte, readbacks, autorité de merge, provenance et sources sont conservés ici et dans le manifeste d’admission.

## Échecs diagnostiques et limites

Les runs 031 et 032 sont conservés comme échecs, jamais comptés comme réussites : 031 a révélé le sous-événement de revue non pris en charge (`422`); 032 a observé une livraison d’ouverture sans réponse HTTP (`status=0`) et n’a pas reçu le delivery correspondant. Le run 033 ajoute un readback bloquant de l’événement `opened` avant de poursuivre vers la revue et le merge.

Ce parcours démontre un adaptateur local déterministe, pas un LLM, AgentOS, Forge Code, RAG ni moteur de mémoire. Un run réussi (`n=1`) ne démontre aucun gain de réussite, temps, intervention ou tokens face à une CI conventionnelle. La fixture n’embarque pas la policy v4 : le SHA est épinglé depuis l’hôte, tandis que les critères protégés sont relancés sur le commit exact. Bubblewrap isole le vérifieur; les services locaux partagent le même UID. Le verrou de merge ne coordonne que les writers coopératifs et Forgejo ne fournit pas de CAS contre un writer externe. Cargo `license` reste `null`, décision non choisie; les droits de redistribution ne sont pas évalués.
