# MISSION-012-COMPOSE-064 — SHA de fusion lié au webhook, aux reçus et au checkout fusionné

Date : 25 octobre 2026. Worktree produit `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`, fortement modifié et préservé. Forgejo local v17.0.0, binaire SHA-256 `e5ec437cd6357e8d1b4580cb47ab1daa58a32b33971a63db63446a535d33adb7`; son commit de build exact demeure inconnu.

## Échec observé puis correction

Le run isolé 063 a échoué après l’effet de merge : son observateur n’avait pas demandé au parseur de valider `expected_merged=True` pour `pull_request/closed`; le parseur omettait alors `merge_commit_sha` et le runner s’arrêtait avec une `KeyError`. Le merge privé local était observable dans le reçu de l’autorité, mais aucune réconciliation complète n’a été revendiquée. Le dossier 063 conserve le constat expurgé et les résultats partiels.

Le correctif marque explicitement l’événement fermé comme fusionné. Le schéma du reçu webhook passe à 5. Le validateur rejette désormais un SHA absent ou différent dans le corps `closed`, l’état canonique ou tout reçu individuel. Le test `test_composed_merged_webhook_requires_merge_sha_in_event_and_current_state` démontre ces refus.

## Parcours composé 064

Commande exécutée dans un état neuf sous `/tmp` :

```sh
python3 scripts/run_mission_012_live_composed_e2e.py \
  --state-dir /tmp/mission-012-composed-merge-sha-064 \
  --forgejo-binary /tmp/forgejo-17.0.0 \
  --evidence-dir "$PWD/evidence/mission-012-live-agent-webhook-local-adapter-064" \
  --execution-id live-mcp-064 \
  --allocation-id mission012-merge-sha-064 \
  --agent-adapter local-test-adapter \
  --sandbox-test-adapter \
  --budget-cap-usd 0.20
```

Le webhook natif Forgejo sur l’issue #1 a été reçu en HTTP 202, puis l’adaptateur local a consommé le contexte admis et modifié uniquement `src/lib.rs`. La base échoue au critère (`-2`, attendu `10`); candidate et revérification pré-merge passent `2/2`. Le vérificateur indépendant est identifié comme `mission-012-rust-isolated-cargo-acceptance-v2` et accepte les commits base/candidate exacts. La PR #2 est fusionnée par `forgejo-reconcile-local-rust`; les essais avant revue et avec le reviewer non autorisé sont refusés en HTTP 405. L’autorité confirme la tête de `main`, le commit de merge et l’arbre candidat égal à l’arbre fusionné.

Le SHA de candidate est `6ed55ce35290e9a1814607e253324a6a88610abf`; le merge est `a71ec6833c236b4c3f0cc56cd90bb1eabf9a9afb`. Les notifications natives `opened`, `approved review` et `closed` ont chacune une tâche HTTP 202 et se réconcilient en `current_state_read`. Le champ `merge_commit_sha` du webhook fermé, le champ canonique et celui des trois reçus sont tous égaux au SHA du merge. Le rapport structuré est [pr-webhook-reconciliation.json](pr-webhook-reconciliation.json); le merge Forgejo et la candidate sont lus dans [forgejo-result.json](forgejo-result.json) et [rust-merge-authority-result.json](rust-merge-authority-result.json).

## Vérification après fusion

Le dépôt nu du Forgejo de test a été cloné avec `--no-hardlinks --no-checkout`. Dans des worktrees détachés propres, la base exacte `81dcd94e0e65e95d2732ff9d3fffcd32b23c69cd` reproduit `1 passed; 1 failed`, avec `-2` au lieu de `10`; le commit de merge exact passe `2/2`. Les SHA-256 du `src/lib.rs` fusionné (`173304ad7d32aabc1e8e1ee41eabda3398a8690798e68f906bcebc73cbcd6d15`) et du fichier de critères d’acceptation concordent. Le merge a les deux parents attendus et le même arbre que la candidate. Les commandes étaient `cargo test --offline --locked` sous systemd et Bubblewrap : limites CPU/mémoire/processus/temps, namespaces utilisateur/montage/PID/réseau, source montée en lecture seule, cibles Cargo vierges, aucun home ou credential hôte.

Le reçu détaillé est [postmerge-fresh-checkout.json](postmerge-fresh-checkout.json), avec [le log rouge de la base](postmerge-base-cargo-test.log) et [le log vert du merge](postmerge-merge-cargo-test.log). La validation est rétrospective et ne reteste pas l’API Forgejo.

## Versions et contrôles

- Python 3.12.3; Cargo 1.95.0; Rust 1.95.0; empreinte du binaire Forgejo ci-dessus.
- Driver PR : 41/41; parseur d’événements Forgejo : 59/59; `py_compile`, `git diff --check` et manifeste local vérifiés.
- SHA-256 des sources enregistrées pour le lot : `run_mission_012_live_composed_e2e.py` `b9f9169ee417ea980d78c390caec49d8ca041752cd4e9161ee5f267bb7f80c66`, driver PR `d310acc42f38459e56f294d99095213867f0e0fce3fe1bf69b185300201d07c5`, tests `823d4c41514656716457309007c8f777a0ad23bcf01f01afc8fc47a1457d34b3`.
- Le snapshot de composition, les commandes et leurs empreintes source sont inclus dans `composition-source-snapshot.zip` et `composed-run.json`.
- Adaptateur : `local-test-adapter`; `model_invoked_during_run=false`; tokens modèle non applicables; coût modèle rapporté `0.00 USD`.

## Limites

Un seul dépôt/issue/PR sur une instance jetable (`n=1`). L’adaptateur est déterministe et ne démontre aucun parcours LLM, AgentOS, RAG, mémoire, CI conventionnelle ni comparaison de productivité. Le résultat n’établit aucun gain général. Le reçu d’autorité indique explicitement que l’exclusion des writers externes n’est pas fournie par le verrou coopératif; cette preuve ne résout donc pas la course entre writers indépendants. Les objets de test Forgejo et le commit de build inconnu limitent la portée. Cargo ne choisit aucune licence (`license=null`, `license_file=null`, `publish=[]`) et les droits restent non évalués.
