# Revue de dérive des sources — CPU-055/056

Date : 24 octobre 2026. Revue indépendante read-only des reçus composés, empreintes de sources, sorties de vérification et manifestes CPU-055/056. Aucun test, service ou processus n’a été lancé.

## Résultat fonctionnel attesté

- CPU-055 montre le parcours local depuis l’événement d’ouverture d’une issue jusqu’à la PR et à la fusion par l’autorité Rust, avec `local-test-adapter`. La base échoue sur le critère de clamp, la candidate passe, et la vérification prémerge/readback sont enregistrés.
- CPU-056 montre le refus après avance concurrente de `main` : `invalid_review_base_binding`, `merge_attempted=false`, `merge_authorized=false`, issue cible encore ouverte.
- Les sorties liées dans le ledger et les quatre manifestes — deux runs et deux admissions — ont été vérifiés. Les empreintes/sorties enregistrées concordent.
- La tâche et les critères sont préparés dans la fixture; la transformation d’une demande libre et l’exécution autonome d’un agent LLM ne sont pas démontrées. L’adaptateur déterministe remplace une chaîne connue par `maximum` dans `scripts/mission_012_test_agent_adapter.py`.

## Dérive de provenance

Les deux objets `composed-run.json` déclarent le même HEAD source et les mêmes empreintes des 15 sources. Onze empreintes concordent avec le workspace courant. Quatre ne concordent pas; les octets exacts correspondants n’ont été retrouvés ni dans le dépôt et ses workspaces `.runtime`, ni parmi les blobs Git atteignables par les refs locales. Trois chemins sont absents de toutes les versions Git atteignables; le quatrième a deux versions atteignables, aucune avec le hash attendu :

| Source | SHA déclaré | SHA courant |
|---|---|---|
| `scripts/run_forgejo_live_webhook_mission_admission.py` | `73f9c3a44fa23b463f7bf02e8101b8c552a321e57139d0fd1d7c5cf28f653da3` | `e809fdb156da54822c9ebb031c4b3d80d68f886ef0adde97ccf7f32ce97c68ec` |
| `scripts/run_forgejo_live_webhook_reconcile_emitted.py` | `bc4ed7d6fe2e00589ac0de39b369d7b7fbf8556b21050a161f3c09d36089eb33` | `360de019cceeaedbf0b18c9b524cbb46ccd10aba1a7b285ee4765a491a4af473` |
| `src/bin/forgejo-reconcile-local.rs` | `a2315f502ffa10df03d0d5a5a1b2a7040d0db7cbf62c85034f071cdad27d2552` | `b7c7cdb66e49d35bdc4c5d723d9f1effeebe3855b1d03c60f730f042b0fe95ee` |
| `src/mission_012_host.rs` | `a067b8919e5537422994919caa5f0a52fd9598e391294e79bd83ac33b1719a29` | `312d9bb7eb135fc9b6bb59c616d47451fbd11f98c9668a192a09f3af9dc5f7c4` |

`source_head` seul n’identifie pas les changements non commités. Les reçus prouvent l’intégrité des artefacts archivés, mais ne suffisent pas à reconstituer exactement ces quatre sources exécutées.
