# Run local 037 — reçus PR strictement liés aux livraisons

Date : 24 octobre 2026. HEAD source observé : `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`, avec de nombreuses modifications locales préexistantes conservées. Les sources, binaires et livrables du parcours sont identifiés dans `composed-run.json` et les receipts. Forgejo `17.0.0+gitea-1.22.0`, binaire `/tmp/forgejo-17.0.0`, SHA-256 `e5ec437cd6357e8d1b4580cb47ab1daa58a32b33971a63db63446a535d33adb7`. Le clone officiel v17 consulté est au commit `0d791421f85ca13a63b665053d81ea765fbbc571`; la provenance de construction du binaire n’est pas établie.

Commande reproductible depuis ce worktree :

```sh
python3 scripts/run_mission_012_live_composed_e2e.py \
  --forgejo-binary /tmp/forgejo-17.0.0 \
  --state-dir /tmp/mission012-composed-adapter-037 \
  --evidence-dir "$PWD/evidence/mission-012-live-agent-webhook-local-adapter-037" \
  --execution-id live-mcp-037 \
  --allocation-id mission012-local-adapter-037 \
  --budget-cap-usd 0.20 \
  --agent-adapter local-test-adapter
```

## Résultat observé

- La chaîne locale Forgejo a lié l’événement `issues/opened` à l’exigence versionnée `MISSION-012-CLAMP-001`, l’allocation `mission012-local-adapter-037` et la base `9b4ed79547e61fc149818aefd0b6db896a31524c`. Un replay d’allocation est refusé comme déjà revendiqué; un credential invalide est refusé avant admission.
- L’adaptateur déterministe a consommé `mission.context`, `workspace.read_text`, `workspace.write_text` et `cargo.test`. Le test d’acceptance échoue sur la base (`-2` au lieu de `10`); après correction, le candidat ne change que `src/lib.rs` et les deux tests d’acceptance passent.
- Le candidat `9ba338f434addbeea2d67222cc3799da5129fc1e` a ouvert la PR locale #2. Le vérificateur indépendant a relancé les critères du commit exact sous Bubblewrap, namespaces PID/réseau, snapshot read-only et Cargo offline/locked. L’API Forgejo refuse la fusion avant revue et une fusion par `verifier-bot` en HTTP 405. La revue approbatrice est relue par API et liée au candidat.
- L’autorité Rust a fusionné et relu `52b421e199896ec3efa3ef03ff44918e78802902`; son parent base, parent candidat, arbre, PR et issue fermée concordent. Trois tâches et notifications webhook PR (`opened`, `pull_request_approved/reviewed`, `closed`) ont chacune HTTP 202, UUID distinct et receipt `current_state_read`. Le producteur compare désormais l’UUID renvoyé par le CLI à celui demandé avant de produire la receipt. Le composeur exige aussi les types entiers exacts des compteurs, du schéma, des numéros PR et des champs numériques des receipts. Le hook est désactivé avec readback, le receiver s’arrête et aucun listener ne subsiste.
- Les tests négatifs couvrent l’UUID CLI absent/différent, schema `4.0`, numéro PR booléen égal à `1`, compteurs flottants égaux, entiers de receipt booléens/flottants, receipts absentes/dupliquées/en échec et état périmé.

## Vérifications

- Parcours Forgejo isolé 037 : succès, `n=1`, avec `local-test-adapter`; aucun LLM appelé et coût de modèle observé `0 USD`.
- `cargo test --offline --locked` : 387 réussis, 0 échec, 14 ignorés, sur 33 groupes. Les scénarios explicitement ignorés parce qu’ils exigent Bubblewrap/réseau isolé restent hors de cette suite; le vérificateur du parcours exécute toutefois l’acceptance candidate dans son sandbox séparé.
- `python3 -m unittest discover -s tests -p 'test_*.py'` : 107 tests réussis. Les 5 tests de régression ciblés passent aussi; `py_compile`, `cargo fmt --all -- --check`, Clippy `--all-targets` avec les trois allowances historiques et `git diff --check` passent.
- Les manifestes SHA-256 du parcours, de son admission et de `evidence/` sont vérifiés après génération. `cargo metadata --offline --locked` confirme encore `agent-forge-core` avec `license=null`.
- Outils observés : rustc/cargo 1.95.0 et Python 3.12.3 (`tool-versions.txt`).

## Limites

Ce parcours démontre le chemin local avec un substitut déterministe, pas un agent LLM, ni un gain mesuré contre Forgejo+CI conventionnelle. AgentOS, Forge Code, RAG et mémoire ne sont pas intégrés. La policy v4 du vérifieur est épinglée par l’hôte, pas versionnée dans le dépôt fixture. Bubblewrap isole le vérifieur mais les services locaux partagent l’UID; le verrou ne protège que les writers coopératifs et Forgejo ne fournit pas de CAS contre un writer externe. Le binaire Forgejo est identifié par empreinte, mais sa construction depuis le clone officiel n’est pas attestée. Aucun droit de réutilisation GitSpace n’est conclu; `license=null` reste inchangé.

Prochaine tranche : simuler une interruption réelle du processus entre POST de merge et readback, puis reprendre par relecture Forgejo sans répéter un effet ambigu. La comparaison d’efficacité attendra un agent réellement autorisé et un budget de coût fiable.
