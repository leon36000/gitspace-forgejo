# COMPOSITION-SOURCE-SNAPSHOT-001 — sources composées archivées et vérifiées

Date : 24 octobre 2026. Dépôt de test local, HEAD observé `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. Aucun commit, publication ou changement de production.

## Changement

La revue des runs CPU-055/056 a trouvé quatre empreintes de sources qui ne correspondent plus au workspace et dont les octets exacts n’ont pas été retrouvés dans les refs locales. Le hash de HEAD seul ne capturait pas les modifications non commitées.

Le nouveau module `scripts/composition_source_snapshot.py` découvre et archive les entrées locales pertinentes : `Cargo.toml`/`Cargo.lock`, sources Rust, pilotes et tests Python, fixtures JSON ainsi que les configurations de build présentes. Il exclut explicitement `evidence/`, `.runtime/`, `.claude/`, `.git/`, `target/`, caches Python et dépendances Node. Il borne les sources à 64 Mio/10 000 fichiers et l’archive à 128 Mio, refuse chemins non canoniques, fichiers non réguliers et liens symboliques, produit un ZIP à dates normalisées avec manifeste interne SHA-256, puis sait vérifier la source et copier l’archive sans écraser une destination.

`run_mission_012_live_composed_e2e.py` capture ces entrées avant l’admission. Il compare leur liste et leurs hashes avant et après chaque étape et ajoute le ZIP, son hash et les hashes par fichier au reçu composé et à son manifeste lorsque le parcours se termine. Une dérive fait échouer le composeur au lieu de publier un reçu positif.

## Vérifications

- `python3 tests/test_composition_source_snapshot.py` : 10 réussis. Les cas couvrent capture/relecture, détection d’une modification ou d’un ajout, lien symbolique direct et parent, chemin `..`, limites de lecture, refus d’écraser, copie intègre et arrêt du composeur lorsqu’une étape modifie une source.
- `cargo test --offline --locked --test webhook_reconcile_contract` : 21 réussis, 0 échec.
- `cargo test --offline --locked --bin forgejo-reconcile-local` : 98 réussis, 0 échec.
- `python3 scripts/test_cargo_license_state.py` : 5 réussis. Le readback `cargo metadata --locked --offline --no-deps` confirme `license=null`, `license_file=null`, `publish=[]`, `license_decision=unselected`, `redistribution_rights=not_assessed`; aucune licence n’est choisie.
- Compilation Python des trois fichiers concernés, `run_mission_012_live_composed_e2e.py --help` et `git diff --check` : code 0.
- Snapshot vérifié après capture : 82 fichiers, 2 500 289 octets; archive SHA-256 `769b1cf1a8cc0c34cd5bcce246c9ea18230fedcfef5f9ba3671504cc5caf2245`; manifeste interne SHA-256 `0b816fded9b7ea4932a413c8fa49493bcb8346270a78ef76444c9dd6d35f4259`. Le ZIP est conservé dans `source-inputs.zip` et sa vérification dans `source-snapshot-verify.log`.

Les commandes et sorties sont conservées dans ce dossier; `SHA256SUMS` couvre les preuves locales. Quatre captures intermédiaires restent identifiées comme telles; le reçu ci-dessus référence uniquement `source-inputs.zip`.

## Portée et limites

Aucun run composé Forgejo, aucun appel de modèle et aucune fusion n’ont été exécutés dans ce lot. La garde du composeur a été testée avec un stage fixture qui modifie une source, mais l’écriture de l’archive dans un résultat complet n’est pas encore exercée de bout en bout. Le snapshot décrit les octets observés au début; l’exécution lit encore le checkout vivant. La comparaison avant/après chaque étape détecte une dérive persistante mais pas une modification puis restauration entre contrôles. Une mutation pendant l’étape PR pourrait être détectée seulement après son effet de fusion; ce garde-fou ne l’empêche pas. Il ne remplace pas une exécution depuis une copie immuable. `Cargo.lock` et les binaires sont liés par empreinte, mais leur disponibilité n’est pas reproduite par le ZIP seul.

Cette capture courante ne répare pas rétroactivement la provenance CPU-055/056. Conserver leurs statuts comme résultats historiques; quatre anciennes versions de fichiers restent introuvables et ne doivent pas être revendiquées comme vérifiées sur les sources présentes.

Le parcours E2E contrôlé antérieur restait un `n=1` avec `local-test-adapter`; les tests ciblés de ce lot ne mesurent aucune amélioration de taux de réussite, interventions, durée, tokens ou coût. Aucun LLM/AgentOS/RAG/mémoire n’est intégré par ce changement.
