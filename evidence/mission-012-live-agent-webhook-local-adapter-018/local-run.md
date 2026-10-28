# Parcours MISSION-012 composé avec l’adaptateur local — run 018

Date : 24 octobre 2026. Dépôt source `agent-forge-core`, HEAD examiné `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`; worktree volontairement conservé tel quel et déjà modifié. Binaire Forgejo local `17.0.0+gitea-1.22.0`, SHA-256 `e5ec437cd6357e8d1b4580cb47ab1daa58a32b33971a63db63446a535d33adb7`.

## Commande

```sh
python3 scripts/run_mission_012_live_composed_e2e.py \
  --state-dir /tmp/mission012-composed-adapter-018 \
  --forgejo-binary /tmp/forgejo-17.0.0 \
  --evidence-dir "$PWD/evidence/mission-012-live-agent-webhook-local-adapter-018" \
  --execution-id live-mcp-018 \
  --allocation-id mission012-local-adapter-018 \
  --budget-cap-usd 0.20 \
  --agent-adapter local-test-adapter
```

Le plafond figure dans le schéma commun, mais ce mode n’appelle aucun modèle et ne consomme aucun budget API. L’exécuteur déterministe reprend `scripts/mission_012_test_agent_adapter.py`, déjà présent dans le prototype local; la copie canonique est indépendante de GitSpace et son hash est inclus dans `composed-run.json`.

## Résultat observé

- **Demande/admission :** Forgejo local a créé l’issue privée #1 et émis un vrai hook `issues/opened`; la tâche de livraison Forgejo et l’entrée du receiver HMAC correspondent au même UUID et reçoivent HTTP 202. L’admission versionnée relie l’allocation `mission012-local-adapter-018`, l’exigence SHA-256 `3a3c444075c93a3bcfb2bec17d1a2e74ec0051effc2994e6b02ac095eb0dd431` et la base `4e8b2887044a1fdb6bd24deb8a663a457b2a8896`. Le credential hôte invalide est refusé avant création du claim; le rejeu retourne `already_claimed` sans modifier les empreintes. Le hook est désactivé avant l’arrêt.
- **Contexte et changement :** l’adaptateur appelle les outils MCP exposés par l’hôte, consomme la demande admise, observe l’acceptance rouge, lit le code et les critères autorisés, puis écrit uniquement `src/lib.rs` par MCP. Le candidat est lié au reçu hôte `e72cd4f268a20aa0bf03be6e20fecd9def66e61100dafbf7607efa2f0b2fb3a6` et au SHA du binaire MCP `c7fa6ab292e7151ae1f97dfb8d860ca36af0260b7cf3704ba0c2600df13d8c48`.
- **Échec puis correction vérifiés séparément :** sur la base `4e8b2887044a1fdb6bd24deb8a663a457b2a8896`, l’acceptance échoue avec exit 101 (`left: -2`, `right: 10`). Sur le candidat `88d2ebf0698a46bcd3eee89cf334dd65e238643a`, l’acceptance passe (2 tests) sous Bubblewrap, en mode offline/locked, snapshot en lecture seule et sans credentials hôte. Les journaux `base-red-cargo-test.log` et `candidate-green-cargo-test.log` sont conservés ici.
- **PR/vérification/fusion :** PR #2 dans le dépôt privé; le vérificateur isolé `local-isolated-cargo-acceptance-v1` retourne `pass` pour le commit candidat exact. Une fusion sans approbation est refusée HTTP 405; l’identité reviewer de fixture `verifier-bot` n’a pas le droit de fusionner (405). Après vérification et approbation liées au même SHA, le pilote de test appelle directement l’API Forgejo `POST /merge` avec le compte owner local; Forgejo confirme le commit de fusion `6aae491b0ee55da917edecddb972e7c7e61bc901`, l’issue fermée et `main` à ce commit. Ce run ne passe pas par l’autorité de fusion Rust. Le compte owner local contrôle donc cette fusion de test; ni lui ni `verifier-bot` ne sont une autorité de production ou une revue humaine.
- **Usage du modèle :** `agent_adapter=local-test-adapter`, `model_invoked_during_run=false`, coût modèle `0.0`, effort `deterministic`; aucune invocation Claude/LLM ou aucun sous-agent payant n’a eu lieu dans ce run.
- **Nettoyage :** le driver rapporte receiver et processus Forgejo arrêtés; le contrôle après run ne voit aucun listener sur `127.0.0.1:3130` ou `:3132`. Le state de test `/tmp/mission012-composed-adapter-018` (mode 0700) et le journal du pipeline (mode 0600) sont conservés, pas supprimés.

## Preuves et limites

`composed-run.json` lie admission, reçu, candidat, vérificateur, PR et fusion; `composed-stage-summaries.json`, `forgejo-result.json`, `independent-verifier.json`, `pre-review-merge-refusal.json`, `premerge-reverification.json`, les journaux rouge/vert, `contract-suites.log` et `SHA256SUMS` sont co-localisés. `sha256sum -c SHA256SUMS` passe depuis ce dossier. Les hashes des sources utilisés par le composite sont dans `composed-run.json`; `source-hashes.txt` couvre les sources, tests et ce checkpoint.

Le vérificateur ne peut pas employer `mission-012-verify-local` sur cette fixture parce que le dépôt Forgejo ne contient pas `policy/active.json` / policy v4; il teste l’acceptance protégée du commit exact dans un sandbox séparé et lie un hash de politique épinglé par l’hôte. Pour cette raison, le gate de fusion Rust n’est pas appelé : le pilote hôte local valide son résultat puis utilise directement `POST /merge`. La frontière locale partage encore l’UID hôte. L’adaptateur code le correctif de ce fixture précis : ce run ne prouve ni capacité générale d’un LLM, ni intégration AgentOS/RAG/mémoire, ni avantage mesuré. Aucun gain de réussite, de temps, d’interventions, de tokens ou de coût n’est établi par `n=1`.
