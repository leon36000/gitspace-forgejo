# MISSION-012 — sorties brutes liées aux reçus de vérification

Date : 24 octobre 2026  
HEAD observé : `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`  
Périmètre : fermer la réserve de la revue CPU ledger 002 concernant les sorties des étapes vertes.

## Changement

La revue lecture seule a confirmé le lien admission/mission, la validation de l'échec base-red et la persistance du log pré-fusion. Elle a relevé qu'au niveau du reçu composé, les étapes vertes n'étaient pas reliées à leurs octets bruts : le validateur ne contrôlait que le code de sortie et la forme de leur SHA-256.

Le pilote PR place maintenant les octets base64 des trois étapes (`base-red`, `candidate-green`, `premerge-green`) dans les reçus. Le validateur composé les décode, impose une limite de 2 Mio par sortie, recalcule le SHA-256 et vérifie les marqueurs de l'échec initial ou les deux tests d'acceptance verts attendus. La limite du JSON d'étape passe à 10 Mio pour transporter ces trois sorties bornées avec leurs autres champs; les trois journaux restent aussi présents séparément dans le dossier de preuves et couverts par son manifeste. La revue a aussi relevé un message d'erreur indiquant encore 1 Mio; il est désormais calculé depuis la borne réelle de 10 Mio.

## Vérifications

- Le test d'acceptation d'un reçu vert complet et le cas négatif d'une sortie sans les deux marqueurs ont reproduit l'écart avant correction. Après correction, les deux cas, le test de SHA incohérent et les suites concernées passent.
- `python3 -m unittest tests.test_mission_012_live_agent_pr_e2e tests.test_mission_012_live_composed_e2e` : 77 réussis. Les refus ciblent aussi le base64 malformé, les sorties décodées de plus de 2 Mio et le message de la borne JSON à 10 Mio. [Journal](mission-012-live-agent-webhook-cpu-ledger-003-tests.log).
- `python3 -m py_compile` sur les deux pilotes et leurs tests : code 0. [Journal](mission-012-live-agent-webhook-cpu-ledger-003-pycompile.log).
- Run positif 055 avec les sources finales : Forgejo local v17 a fusionné le candidat par l'autorité Rust; statut `merged_in_private_sandbox`. Base `a440f6e997b5b1568cc5a1a73ea599c928d74d83`; candidat `6ca9f45e9fde7d58d7a58ef15ff93a03ad9c8b5e`; fusion `4ff3108b0a597037b8bd2b0675eb8b362c0ab786`; trois étapes de vérification, cinq réservations résolues, 12 574 ms imputés.
- Run 056 avec avance concurrente : `main` passe à `8e7cc52cc53c1d0e1fe0de68941fb5eaca7566a9`; statut `refused_after_concurrent_base_advance_before_merge`. Base `ccac43322a8b89882394f40dac5e08b97523e231`; candidat `3963110ebf3a3a39c04070d20c551aa96e5ce627`; erreur `invalid_review_base_binding`, aucune autorisation ni tentative de fusion, issue encore ouverte; cinq réservations résolues, 12 694 ms imputés.
- Dans chaque run, les octets `base-red` (1 095 octets), `candidate-green` et `premerge-green` (781 octets chacun) correspondent au SHA de leur reçu. Au contrôle initial du rapport, les empreintes des sources composées correspondaient aux fichiers présents. Les manifestes de 12 fichiers de chaque run et de 10 fichiers de chaque admission passent.

Les deux exécutions utilisent `local-test-adapter`; aucun LLM n'a été appelé et le coût modèle rapporté est nul. Le vérificateur indépendant n'a trouvé aucun défaut bloquant; les tests dédiés couvrent aussi la sortie malformée et la limite de décodage. Journaux externes : [run 055](mission-012-live-agent-webhook-cpu-055-runner.log), [run 056](mission-012-live-agent-webhook-cpu-056-runner.log). Reçus et manifestes : [run 055](mission-012-live-agent-webhook-cpu-055/composed-run.json), [manifest 055](mission-012-live-agent-webhook-cpu-055/SHA256SUMS), [run 056](mission-012-live-agent-webhook-cpu-056/composed-run.json), [manifest 056](mission-012-live-agent-webhook-cpu-056/SHA256SUMS). Les répertoires d'admission correspondants contiennent chacun leur `provenance.json` et leur `SHA256SUMS`.

## Limites et suite

Ces exécutions valident une fixture locale, pas une qualité générale ni un gain de temps, tokens ou coût. Le plafond de 10 Mio s'applique à chacune des trois sorties d'étape; le journal agrégé peut donc atteindre environ 30 Mio. Chaque sortie brute de vérification est limitée à 2 Mio; les trois sorties base64 du reçu PR tiennent ainsi sous la borne de son étape. Le ledger calcule une borne CPU depuis le temps d'accounting et le quota imposé, pas une mesure matérielle directe. Les services du bac à sable partagent l'UID hôte et Forgejo n'offre pas de CAS atomique contre un writer externe non coopératif.

## Addendum — dérive des sources observée au 24 octobre 2026

Une revue indépendante read-only a recalculé les 15 empreintes de `composition_source_sha256` dans CPU-055 et CPU-056. Onze concordent encore avec le workspace; quatre diffèrent. Une recherche des octets correspondants dans le dépôt, ses workspaces `.runtime` et les blobs Git atteignables par les refs locales n’a retrouvé aucun snapshot des quatre versions exécutées. `source_head` fixe seulement HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`; il ne capture pas les changements non commités. Pour trois chemins, aucune version n’existe dans cet historique; le quatrième a deux blobs atteignables, aucun ne correspondant au hash attendu. Les reçus des runs et leurs manifestes restent intacts, mais l’affirmation initiale ne décrit plus l’état courant et la reproduction exacte de ces versions n’est pas possible à partir des artefacts conservés et refs locales.

| Source | SHA déclaré par les runs | SHA actuel | État |
|---|---|---|---|
| `scripts/run_forgejo_live_webhook_mission_admission.py` | `73f9c3a44fa23b463f7bf02e8101b8c552a321e57139d0fd1d7c5cf28f653da3` | `e809fdb156da54822c9ebb031c4b3d80d68f886ef0adde97ccf7f32ce97c68ec` | différent, snapshot introuvable |
| `scripts/run_forgejo_live_webhook_reconcile_emitted.py` | `bc4ed7d6fe2e00589ac0de39b369d7b7fbf8556b21050a161f3c09d36089eb33` | `360de019cceeaedbf0b18c9b524cbb46ccd10aba1a7b285ee4765a491a4af473` | différent, snapshot introuvable |
| `src/bin/forgejo-reconcile-local.rs` | `a2315f502ffa10df03d0d5a5a1b2a7040d0db7cbf62c85034f071cdad27d2552` | `b7c7cdb66e49d35bdc4c5d723d9f1effeebe3855b1d03c60f730f042b0fe95ee` | différent, snapshot introuvable |
| `src/mission_012_host.rs` | `a067b8919e5537422994919caa5f0a52fd9598e391294e79bd83ac33b1719a29` | `312d9bb7eb135fc9b6bb59c616d47451fbd11f98c9668a192a09f3af9dc5f7c4` | différent, snapshot introuvable |

Les hashes des reçus d’étape et les manifestes des runs ont été revérifiés par l’agent de revue, qui n’a lancé aucun test ni service. Voir le [relevé indépendant](mission-012-live-agent-webhook-cpu-ledger-003-source-drift-review.md). Il faut conserver CPU-055/056 comme résultats historiques du bac à sable, sans les utiliser comme preuve que ces quatre fichiers actuels étaient exécutés ni revendiquer une provenance source complète.

Observation passive de Claude PID `1947069` à deux points espacés de 32 s : état `Ssl+`; âge `10:07:55` → `10:08:27`; CPU `00:03:49` → `00:03:50`; mtime du marqueur Ralph toujours `2026-10-23 21:00:03.695919020 -0400`. Ces métadonnées ne suffisent pas à conclure à un progrès utile ou à un blocage. Aucun argv, prompt, environnement, log ou transcript n'a été lu; le processus n'a pas été contacté, arrêté ou relancé. L'interface de sous-agents disponible ne propose pas Sonnet 5.5. Aucun nouvel appel Claude n'a été engagé dans ce lot; le heartbeat de continuation existant reste actif.

Suite : reprendre les lacunes d'intégration restantes et la baseline appariée lorsque le budget modèle sera réellement mesurable.
