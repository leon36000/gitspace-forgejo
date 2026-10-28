# COMPOSITION-SOURCE-SNAPSHOT-002 — écriture du snapshot dans le reçu final

Date : 24 octobre 2026. Worktree local `local/WEBHOOK-005-canonical-links`, HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. Aucun commit, publication, changement de production ou appel de modèle.

## Lacune ciblée

Le lot 001 testait la capture, la validation avant/après les stages et le refus d’écraser une archive, mais ne démontrait pas que `run()` persistait réellement le ZIP, sa map d’empreintes et le hash de l’archive dans le reçu composé et `SHA256SUMS`.

## Vérification ajoutée

`tests/test_composition_source_snapshot.py::test_composed_runner_persists_source_archive_in_final_receipt_and_manifest` invoque le vrai `run()` dans un dépôt temporaire. Les stages, les validations métier de stage et le readback Forgejo sont des fixtures synthétiques; aucun service Forgejo ni agent n’est lancé. La capture/validation des sources, la copie hashée, la sérialisation exclusive du reçu et le producteur de manifeste de preuves restent les implémentations réelles.

Le test relit `composed-run.json`, ouvre l’archive pour lire son manifeste interne, compare la map d’empreintes du reçu à cette source, puis vérifie que `SHA256SUMS` couvre l’archive, le reçu final et le readback. La fixture ne simule pas de mutation des sources pendant ce chemin; le test séparé du lot 001 garde la preuve du refus après mutation par un stage.

## Vérifications fraîches

- `python3 tests/test_composition_source_snapshot.py` : 11 réussis, 0 échec, sous Python 3.12.3. Journal : [source-snapshot-tests.log](source-snapshot-tests.log).
- `python3 -m py_compile scripts/composition_source_snapshot.py scripts/run_mission_012_live_composed_e2e.py tests/test_composition_source_snapshot.py` : code 0. Journal : [pycompile.log](pycompile.log).
- `python3 scripts/run_mission_012_live_composed_e2e.py --help` : code 0. Journal : [runner-help.log](runner-help.log).
- `git diff --check` : code 0 au contrôle ciblé initial; un contrôle final après modification des documents est conservé dans [diff-check.log](diff-check.log).

Les octets du ZIP restent éphémères dans le dossier temporaire de test; le reçu de fixture atteste leur hash, pas la production d’un nouvel artefact E2E. Le code du runner n’a pas été modifié par ce lot.

## Portée et limites

**Testé isolément :** invocation du finaliseur du runner avec stages synthétiques, copie du vrai snapshot, reçu JSON et manifeste SHA-256. **Non testé de bout en bout dans ce lot :** Forgejo, webhook, admission, agent, PR, vérification indépendante, fusion, readback réel ou exécution depuis une copie immuable. Il ne couvre pas non plus une mutation suivie d’une restauration entre les contrôles du composeur. Aucun appel de modèle, coût, performance ou comparaison Forgejo+CI conventionnelle n’est établi.

Le parcours Forgejo local historique reste distinct et utilise `local-test-adapter`; cette frontière ne permet pas de le présenter comme un parcours LLM ou de conclure à un gain général. Voir aussi [COMPOSITION-SOURCE-SNAPSHOT-001](../composition-source-snapshot-001/local-run.md) pour l’archive source antérieure et sa portée.
