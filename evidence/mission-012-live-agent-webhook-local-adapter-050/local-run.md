# Mission 012 composée 050 — conflit même fichier avec validation durcie

Date : 25 octobre 2026. Dépôt produit au HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`, branche `local/WEBHOOK-005-canonical-links`. L’exécution revalide le parcours 049 après extraction du contrôle de preuve concurrent dans un prédicat testé séparément; le snapshot source du composeur est donc celui du code durci.

Commande exécutée :

```sh
python3 scripts/run_mission_012_live_composed_e2e.py \
  --state-dir /tmp/mission012-same-file-conflict-050 \
  --forgejo-binary /tmp/forgejo-17.0.0 \
  --evidence-dir evidence/mission-012-live-agent-webhook-local-adapter-050 \
  --execution-id live-mcp-007 \
  --allocation-id mission012-same-file-conflict-050 \
  --budget-cap-usd 0.20 \
  --agent-adapter local-test-adapter \
  --inject-concurrent-code-conflict
```

Forgejo `17.0.0+gitea-1.22.0` (binaire SHA-256 `e5ec437cd6357e8d1b4580cb47ab1daa58a32b33971a63db63446a535d33adb7`), Python 3.12.3 et Git 2.43.0. La capture de provenance liée à ce run compte 83 fichiers et 2 598 521 octets; archive SHA-256 `9d070e4bf823dfaf5cb529478c6a4fba0f69d1dfddae1adb7a29b2a2f3ee843c`, manifeste SHA-256 `e97dc0e1a5f630a45d849f27925125beb2a3adae37656a2d40e132d05e7200e4`.

Résultat composé : `refused_after_concurrent_code_conflict_before_merge`, code de sortie 0. L’admission lie l’exigence SHA-256 `3a3c444075c93a3bcfb2bec17d1a2e74ec0051effc2994e6b02ac095eb0dd431`; la politique est liée au SHA-256 `e23907b5bd6b2c0f342c88ddcfa9f9a9be98065adcd24f9f7299dc796b5739eb`. Le commit candidat est `1cbe6a88152af40bf4ecdf744d3a914330a290d0` et la source corrigée a le SHA-256 `173304ad7d32aabc1e8e1ee41eabda3398a8690798e68f906bcebc73cbcd6d15`.

Le test de base échoue sur la valeur supérieure (`left: -2`, `right: 10`, code 101 attendu). Les deux tests d’acceptation passent ensuite; le vérificateur isolé répète la vérification avant décision. Une PR concurrente approuvée modifie la même ligne de `src/lib.rs` et fusionne seule. `git merge-tree --write-tree --name-only -z refs/heads/main refs/heads/agent/mission-012-live-agent-webhook-local-adapter-050` retourne 1 et signale uniquement `src/lib.rs`; le hash de sortie est `9069fd8d4f7f4498ad2949e27b5be0efc777eaad3c8b34c702bd61b294ca1d96`.

L’autorité Rust rejette la PR candidate #2 avec `invalid_review_base_binding`, `merge_authorized=false` et `merge_attempted=false`; cette PR reste ouverte et non fusionnée. La PR concurrente #3 est fusionnée avec HTTP 200; `main` pointe sur `4ef215c2cf349698dc77bf448d9cb3d904e5d095` et la source concurrente a le SHA-256 `15435b094b716996fa9928d0f20ca4362f9b4fad219bc1dd3c922d0c27ade37c`. La vérification de readback confirme le commit, le chemin en conflit, le candidat resté non fusionné et le reçu concurrent complet.

Après ce refactoring, la suite ciblée passe 32/32; `py_compile` passe et le composeur frais 050 retourne 0. `local-test-adapter` n’invoque aucun modèle; coût déclaré 0,00 USD. Le plafond de 0,20 USD est configuré mais non dépensé. Ce test est une composition locale séquentielle; il ne prouve pas des écritures réellement simultanées, ne protège pas contre un writer non coopératif et n’est pas une comparaison avec une CI conventionnelle. Le parcours positif de fusion demeure le run 046.
