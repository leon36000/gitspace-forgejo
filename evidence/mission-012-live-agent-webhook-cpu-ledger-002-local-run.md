# MISSION-012 — durcissement des preuves du ledger CPU

Date : 24 octobre 2026  
HEAD observé : `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`  
Périmètre : traiter trois écarts relevés pendant la revue indépendante du parcours ledger CPU.

## Changements

- Le pilote PR valide le format du `mission_id` admis avant de démarrer Forgejo. Il compare l'identité de chaque mesure à celle de l'admission et contrôle le ledger final ainsi que ses étapes avant de préparer ou d'appeler l'autorité Rust.
- Le validateur composé exige le code Cargo 101 pour l'échec initial, décode les octets de sortie, recalcule leur SHA-256 et vérifie l'assertion attendue, les valeurs `-2` et `10`, le test et le résumé `1 passed; 1 failed`.
- Le pilote conserve la sortie Cargo pré-fusion dans `premerge-reverification-cargo-test.log`; le JSON nomme cette sortie et en porte l'empreinte. Le manifeste du run couvre le fichier.

Les nouveaux cas négatifs ont d'abord échoué avec l'ancienne implémentation, puis passent après correction. Les fichiers restent non suivis par Git par rapport au HEAD observé; les runs composés enregistrent leurs empreintes de sources.

## Vérifications

- Suites ciblées : `python3 -m unittest tests.test_mission_012_live_agent_pr_e2e tests.test_mission_012_live_composed_e2e` — 71 tests réussis. [Journal](mission-012-live-agent-webhook-cpu-ledger-002-tests.log).
- `py_compile` des deux scripts et des deux fichiers de tests : réussi. [Journal](mission-012-live-agent-webhook-cpu-ledger-002-pycompile.log).
- Run positif 051 sur Forgejo local v17 : admission réelle via `issues/opened`, adaptateur MCP déterministe, échec initial d'acceptance, candidat et re-vérification pré-fusion verts, fusion par l'autorité Rust et lecture canonique du résultat. Base `e9f3dff00dc7b10fd388be9c6667eb5e89e0e1bc`; candidat `205b136cd8185ec0a4e86da00b5280df440ff7d4`; fusion `2768c7bfacebde25b16da1ed1a821cb324095891`; 12 658 ms imputés à cinq réservations CPU résolues. SHA-256 du résultat composé : `6d952e6ac543b2b2bd274950ad5740bd4a40766113b790d67d7a54062f3d3dda`.
- Run 052 avec avance concurrente de `main` : refus avant fusion, code `invalid_review_base_binding`, aucune autorisation ni tentative de fusion; l'issue reste ouverte. Base `fe9e8f5edd346b7d42e363565dadc7ec9d481ce5`; candidat `0e7aa7053963122571174a753f0fd641484d892c`; nouvelle base `8c7fade164a5c3533ea58a2d7c8f73bc87ec82fd`; 12 732 ms imputés à cinq réservations résolues. SHA-256 du résultat composé : `0f3769041a24e176b602d200dd4135fbf3e9b3ff196dce1cdcae085f2453b1f4`.
- Dans les deux runs, le `mission_id` est lié à l'admission et les octets base-red portent l'échec d'acceptance attendu. Le journal pré-fusion brut fait 781 octets; son empreinte correspond au reçu JSON et figure dans le manifeste. Les manifestes run/admission des deux exécutions ont été vérifiés.

Les commandes de reproduction utilisent `scripts/run_mission_012_live_composed_e2e.py`, Forgejo `/tmp/forgejo-17.0.0`, le profil explicite `local-test-adapter` et des dossiers d'état dédiés sous `/tmp`. Les reçus détaillés sont [run 051](mission-012-live-agent-webhook-cpu-051/composed-run.json), [run 052](mission-012-live-agent-webhook-cpu-052/composed-run.json) et le [manifest général](SHA256SUMS).

## Limites et suite

Une fixture et un adaptateur déterministe ont été utilisés; aucun LLM n'a été appelé, le coût modèle rapporté est nul. Ces essais ne mesurent ni gain général de qualité, ni temps, ni tokens, ni coût. Les services locaux partagent l'UID hôte; Forgejo ne fournit pas ici de comparaison atomique de base contre un writer externe non coopératif.

Observation passive de Claude, PID `1947069`, à deux points espacés de 33 s : état `Ssl+`, âge `09:56:20` → `09:56:53`, CPU inchangé à `00:03:46`; le mtime du marqueur Ralph reste `2026-10-23 21:00:03.695919020 -0400`. Ces métadonnées ne prouvent ni progrès utile ni blocage. Aucun argv, prompt, environnement, log ou transcript n'a été lu; le processus n'a pas été contacté, arrêté ou relancé. Le plafond de coût CLI observé auparavant est estimatif, pas une borne ferme. L'interface de sous-agents disponible ne permet pas de choisir Sonnet 5.5; la revue indépendante de ce lot a été confiée à un sous-agent Codex. Le heartbeat de continuation existant reste actif.

Prochaine étape : relier aussi les octets bruts des vérifications vertes à leurs reçus composés, puis poursuivre les lacunes d'intégration. Une baseline appariée Forgejo + agent + CI conventionnelle attend un budget de modèle mesurable.
