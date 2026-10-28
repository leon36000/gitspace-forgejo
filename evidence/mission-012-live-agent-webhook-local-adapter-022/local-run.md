# Parcours MISSION-012 composé avec autorité de fusion Rust — run 022

Date : 24 octobre 2026. Source `agent-forge-core`, HEAD examiné `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`; worktree déjà modifié, laissé intact. Forgejo `17.0.0+gitea-1.22.0`, binaire SHA-256 `e5ec437cd6357e8d1b4580cb47ab1daa58a32b33971a63db63446a535d33adb7`.

## Commande

```sh
python3 scripts/run_mission_012_live_composed_e2e.py \
  --state-dir /tmp/mission012-composed-adapter-022 \
  --forgejo-binary /tmp/forgejo-17.0.0 \
  --evidence-dir "$PWD/evidence/mission-012-live-agent-webhook-local-adapter-022" \
  --execution-id live-mcp-022 \
  --allocation-id mission012-local-adapter-022 \
  --budget-cap-usd 0.20 \
  --agent-adapter local-test-adapter
```

## Résultat observé

- **Demande et admission :** Forgejo a émis un vrai événement `issues/opened` livré au receiver local avec le même UUID et HTTP 202. L’admission lie l’allocation, l’exigence SHA-256 `3a3c444075c93a3bcfb2bec17d1a2e74ec0051effc2994e6b02ac095eb0dd431` et la base `50002db095b7b9265b0ef62ee5000f2b00fd6ebf`. Le credential hôte invalide est refusé sans créer le claim; le rejeu est `already_claimed` sans changement des empreintes.
- **Contexte et changement :** l’adaptateur déterministe consomme le contexte admis par les outils MCP, exécute l’acceptance de la base (échec attendu, exit 101), lit le code, puis écrit uniquement `src/lib.rs`. Le candidat est `086aef6246e7f1ea1b1f0a6453de4483adfc9bff`; le hash du fichier modifié est `173304ad7d32aabc1e8e1ee41eabda3398a8690798e68f906bcebc73cbcd6d15`. L’acceptance du candidat passe (2 tests) dans Bubblewrap, offline/locked, avec snapshot en lecture seule et sans credentials hôte.
- **PR, vérification et fusion contrôlée :** PR #2 est revue par l’identité synthétique `verifier-bot`. L’absence d’approbation et la tentative de fusion directe par le reviewer sont refusées HTTP 405. Le vérificateur `local-isolated-cargo-acceptance-v1` rend `pass` pour le candidat exact; une nouvelle exécution de l’acceptance a lieu immédiatement avant le merge gate. Le pilote construit les binaires depuis les sources locales avec Cargo `--offline --locked`, puis appelle `forgejo-reconcile-local --merge-local-test`; le token owner passe par un pipe hérité privé, jamais par argv ou environnement. L’autorité Rust retourne `merged`, HTTP 200 et un readback liant la base `50002db…`, la tête `086aef…`, le commit de fusion `8c52e2c1909234553a236b60355e2c5b2a94d5d4`, ses deux parents, `main` au même commit et l’issue fermée. Le résultat Forgejo confirme le même commit.
- **Manifestes et empreintes :** le hash de la fixture de politique versionnée v4 est `e23907b5bd6b2c0f342c88ddcfa9f9a9be98065adcd24f9f7299dc796b5739eb`. Le gate reçoit une projection compacte spécifique à son schéma, SHA-256 `a9b52c9d3504a633ed5b9e18df6f479ab54912f0d52c20c55b227ccf2fb148fd`; le hash du binaire `forgejo-reconcile-local` construit pour ce run est `27714d428ee7863356e9583e3b95167c320f8159d927cd6a94d2cddb05080f19`. Les deux fichiers `SHA256SUMS` locaux couvrent le run et l’admission et sont vérifiés.
- **Échecs d’intégration conservés :** run 019 bloqué après build parce que Cargo avait lié ses artefacts par inode; le runner les copie maintenant vers un staging privé indépendant. Runs 020/021 refusés avant le POST : le jeton sans `read:user` ne pouvait pas lire `/user`; le résultat 021 identifie `stage=authenticated-actor-read`. Le runner demande maintenant `read:user,write:repository,write:issue`, sans droit admin, pour son compte local de test. Les trois états et journaux d’échec sont conservés sous `/tmp` et leurs dossiers de preuve; le run 022 est frais.
- **Modèle et nettoyage :** `agent_adapter=local-test-adapter`, `model_invoked_during_run=false`, coût modèle `0.0`; aucun LLM n’est démontré. Le hook est désactivé et les processus/ports de fixture sont arrêtés selon les readbacks.

Les preuves principales sont `composed-run.json`, `composed-stage-summaries.json`, `forgejo-result.json`, `independent-verifier.json`, `rust-merge-authority-build.json`, `rust-merge-authority-result.json`, `pre-review-merge-refusal.json`, `premerge-reverification.json` et les logs rouge/vert. `composed-run.json` contient aussi les hashes des cinq scripts de composition; le document de build contient les hashes des sources Rust et des deux binaires.

## Limites

Ce parcours valide une seule fixture locale avec un adaptateur déterministe codé pour corriger ce cas; il ne démontre ni Claude/LLM, ni intégration AgentOS, Forge Code, RAG ou mémoire, ni gain général. Le vérificateur exécute l’acceptance protégée du commit exact mais projette la policy versionnée hôte vers le petit schéma du gate, car le dépôt Forgejo de test ne contient pas `policy/active.json`; l’autorité Rust ne consomme pas encore l’entrée du journal d’admission comme précondition de fusion. Les identités Forgejo sont synthétiques. Le verrou de fusion est coopératif et ne fournit pas de CAS Forgejo contre un writer non coopérant. L’isolation locale partage le même UID et le build Cargo exécute les scripts de build des dépendances verrouillées dans cet environnement; ce n’est pas une frontière de production. `n=1` ne mesure aucune réduction d’interventions, de temps ou de coût.

Vérifications de la tranche : 42 tests Python ciblés; `cargo test --offline --locked --all-targets -q` passe avec les ignorés prévus par la suite; `py_compile`, `cargo fmt --all -- --check` et `git diff --check` passent. Aucun commit, push, déploiement ou publication n’a été fait.
